# Monaco parity audit 4 (2026-10-12)

A fourth audit, done at `ade2889` after every issue from [`MONACO_AUDIT_3.md`](MONACO_AUDIT_3.md) (#323–#357) was fixed. Eight read-only audits compared the port with the vendored Monaco source, one per area: editing and cursors, view rendering, widgets and menus, navigation and scrolling and folding, the diff editors, options and themes and languages, the public API, and input, accessibility and keybindings. Each finding was checked by reading both sides. §C.1 and §C.5 were also confirmed in the playground, the failing ignored test behind §A.1 was run, and the diff editor, multi diff editor and diffing tests were run (78, 30 and 2 tests, all passing). Mouse drags and screen readers weren't scripted, so those findings come from reading the code.

Most audit 3 fixes hold. These were found incomplete; each follow-up is listed below:

- Audit 3 §C.16: the widget uses Monaco's `canResolve`, but the exported `suggestionCanResolve` and `suggestionMakesTextEdit` context keys don't (here §C.12).
- Audit 3 §D.13: the minimap draws the fold band but not the inline mark, and only on the header's last wrapped piece (here §B.12).
- Audit 3 §E.11: `diffEditorAutomaticRenderSideBySide` ignores screen reader mode (here §E.3).
- Audit 3 §E.16: the inline deleted-lines zone still has no high-contrast borders (here §E.10).
- Audit 3 §E.20 and §E.24: a multi diff view state restored before or with loading is lost, and focus doesn't follow it (here §E.21).
- Audit 3 §E.22: per-file options can't carry editor options such as read-only (here §E.22).
- Audit 3 §F.5: bracket pair colorization still uses prefix checks for strings and comments (here §F.4).

Others follow up a fix that held: §B.1 (exposed by audit 3's §B.4/§B.5 `showIfCollapsed` change), §C.32 (audit 3 §C.23 fixed opening off screen, not scrolling away afterwards), §C.47 (audit 3 §C.50), §D.8 (the `parseInt` root of audit 3 §C.42) and §E.24 (audit 3 §E.19).

The audits overlapped in places. Each shared finding is listed once below, in the section that fits it best, and says which other audit found it too. Left out:

- what [`book/src/differences.md`](../book/src/differences.md) documents as a deliberate difference with a reason that holds;
- what [`MONACO_PARITY.md`](MONACO_PARITY.md) lists as not ported with a reason that holds;
- tracked work: the GPUI-blocked items (#231–#233, #305–#309), publishing to crates.io (#57) and the performance investigation (#378);
- findings already fixed on `main`, and code-style differences with no observable effect.

`vs/platform` and most of `vs/base/browser` aren't vendored. Findings that rely on that code say *upstream, not vendored* and are worked from VS Code or monaco-editor 0.55.1 as they ship.

Paths: the port's are relative to `crates/gpui-monaco/src/` unless they start with `crates/`, `book/` or `docs/`. Monaco's are relative to `vendor/vscode/src/vs/editor/` unless they start with `base/`.

Severity: **P1** visibly broken or wrong behavior; **P2** a missing Monaco sub-feature or a notable fidelity gap; **P3** polish.

## Issues

| Issue | Scope | Sections |
|---|---|---|
| #400 | Focused widgets: editing keys leak from a focused hover, hover buttons and actions from the keyboard, the find widget's Tab cycle | C |
| #401 | Go to Definition and References: the at-cursor check on the selection range, compact references, the alternative command's config, reveal | D, G |
| #402 | Decoration tracking and linked editing: `nodeAcceptEdit` for replaced spans, `collapseOnReplaceEdit`, the default word pattern, reinitializing | A |
| #403 | Accessible diff viewer: focus and screen reader output, state across re-applies and model changes, side by side with a screen reader | E |
| #404 | Diff editor hidden unchanged regions: debug panic, revealed state after an empty list, inline bars, the compact-mode fold glyph | E |
| #405 | Hover: streamed results and "Loading...", no hover under the context menu, glyph hover updates and delay, separators, default range | C, G |
| #406 | Peek references: F4 moves the host, peek mode for Go to References, last-line reveal, `findReferences`, toggling, "open" selects | C, D |
| #407 | Multi diff editor: view state across loading, per-file editor options, a list-wide layout override, kept files with new models | E |
| #408 | Find widget: Enter runs Find Next's action, Cmd+D flashes the find options, a kept widget, the Close tooltip, chrome | C |
| #409 | Model language: `plaintext` for models without one, resolving at creation, canonical ids for names and aliases | G |
| #410 | Decorations overlay and highlights: wrap-point pieces, merged touching decorations, unfocused selection highlights, high-contrast borders, the word highlight cap | B, D |
| #411 | Scrolling and mouse: minimal reveal on click, instant one-line autoscroll, wheel inertia, `revealLine` parsing, focus on clicks | D, H |
| #412 | Commands and keybindings: provider commands run editor actions, `-command` rules, Escape order for the paste chip | G, H |
| #413 | Widget accessibility: names and roles for find, rename, quick input, menus, peek and sticky scroll; the editor's role description | H |
| #414 | Suggest widget behavior: rendering above multi-line ghost text, context keys, keys while loading, status bar labels | C |
| #415 | Links and markdown: model links through editor openers, opening detected links by default, the link provider command, markdown gaps | C, G |
| #416 | Provider changes: recompute links, inlay hints, code lenses and the lightbulb when providers, the language or options change | C, G |
| #417 | Provider requests: formatting indent size, semantic token provider groups, invalid replace ranges, LSP signature help context | G |
| #418 | Marker navigation: F8 in files and the marker zone header | C |
| #419 | Code actions and code lenses: the menu after scrolling, the lightbulb at the selection start and without a glyph margin, lens updates | C |
| #420 | Layout changes: keep the first visible line on wrap, font, zoom and width changes; fractional zoomed line heights; half-width from `n` | B, F |
| #421 | Diff editor high contrast and inline deleted lines: overview colors, dashed borders, full-width tint, render options, the original side's border | E |
| #422 | Input method edits and undo: IME replacements at every selection, outside undo restores selections, Remove Brackets undo steps | A, D |
| #423 | Suggest and parameter hints styling: high-contrast borders, row details, read-more chevron, icon metrics, scrollbar, details scroll | C |
| #424 | Diff editor polish: gutter spans, moved-block arrows, glyph lanes, modified-side overrides, sash ratio, legacy inner changes, aliases | E |
| #425 | Multi diff editor polish: editors for collapsed files, initial height, chevron keyboard access, focus and the R flag | E |
| #426 | View lines and cursors: blink reset, grapheme-wide block cursor, off-screen active bracket guide, advanced wrap indent, whitespace with injected text, NUL | B |
| #427 | Gutter and minimap: the minimap fold highlight, line decorations by view range, glyph codicon size | B |
| #428 | Tokenization: max line length for Monarch and token providers, bracket colorization token types, injected language configuration, factory replacement | F |
| #429 | Quick input and color pickers: Go to Symbol containers and activation, the index-zero toggle, highlight colors, color range tracking, swatch close | C |
| #430 | Inline edits and the paste chip: docked tab icons, warning row, command icons, keybinding labels, paste button anchor | C, G |
| #431 | Docs: decide and document Monaco quirks and VS Code additions; `migrating.md` gaps | B, E, F, G, H |

Several findings share a root cause and are filed together:

- **A focused widget is still inside the editor's key context,** and GPUI matches a binding at any context depth, so editing keys pressed in a focused hover reach the editor (§C.1), and Tab on a focused find control reaches the editor's tab-focus binding (§C.3). #400 covers both, with the hover's keyboard gaps (§C.2).
- **`Patch::transform_offset` isn't Monaco's `nodeAcceptEdit`** (§A.1). It moves every model decoration, including linked-editing ranges, snippet placeholders, find matches and auto-closed ranges. #402.
- **Navigation uses a location's full range where Monaco uses `targetSelectionRange`** (§D.1): the at-cursor check, F12 stepping and the peek's nearest reference.
- **Provider, language and option changes don't trigger a recompute** for links (§G.7), inlay hints and code lenses (§G.8), the lightbulb (§C.36) and linked editing (§A.3). The first three are in #416; linked editing is with its other fixes in #402.

The decide-and-document items (the `lineNumbers: interval` quirk §B.15, the accessible viewer's swapped ranges §E.4, the multi diff sticky header shadow §E.29, tokenizer disposal §F.7, the Insert key §H.5) and the `migrating.md` gaps (§G.17) are tracked by a docs issue, #431. For each, copy Monaco or list the difference in `book/src/differences.md`. Findings whose fix sketch offers documenting as a fallback (§D.3, §E.25, §F.3, §G.6, §G.15) stay with their code issues.

## Counts

138 findings in the eight reports; 135 after merging four duplicates and splitting one finding in two: 4 P1, 35 P2 and 96 P3.

| Section | Findings | P1 | P2 | P3 |
|---|---|---|---|---|
| A. Editing and cursors | 5 | 1 | 2 | 2 |
| B. View rendering | 15 | 0 | 2 | 13 |
| C. Widgets and menus | 47 | 1 | 14 | 32 |
| D. Navigation, scrolling and folding | 10 | 1 | 2 | 7 |
| E. Diff editor and multi diff editor | 29 | 1 | 6 | 22 |
| F. Options, themes and languages | 7 | 0 | 1 | 6 |
| G. Public API | 17 | 0 | 7 | 10 |
| H. Input, accessibility and keybindings | 5 | 0 | 1 | 4 |
| **Total** | 135 | 4 | 35 | 96 |

Merged across audits (each listed once): zoomed line-height rounding (§F.2, rendering and options), the color picker's range after additional edits (§C.30, widgets and API), inlay hints after a provider is unregistered (§G.8, API and widgets) and Go to Symbol containers (§C.26, widgets and API). Split: the diff report's "smaller API gaps" item is §E.20, with its decide-and-document part as §E.4 so the docs issue can carry it.

## A. Editing and cursors

Scope: typing, auto-closing and surrounding, indentation, word operations, multi-cursor, selection and line commands, undo grouping, clipboard, snippets, linked editing, comments and cursor movement.

Checked and matching: the `cursorTypeEditOperations` auto-close path (contained pairs, `typeAndAutoCloseWithUndoStop`, the bracket-balance check, `autoCloseBefore`, quotes after word characters), line and block comments (`_analyzeLines`, insertion and removal, cursor state, `lineCommentNoIndent`), `getPlainTextToCopy` and the clipboard metadata, `PasteOperation` (distribute, `pasteOnNewLine`, block paste), `AutoIndentOnPaste`, snippet `adjustWhitespace`, merge and insert, cursor undo/redo, `cursorMove` units, the multi-cursor limit and the keybinding table. Every command id in `coreCommands`, `linesOperations`, `wordOperations`, `wordPartOperations`, `multicursor`, `comment`, `caretOperations`, `clipboard`, `cursorUndo`, `snippetController2`, `indentation` and `smartSelect` is registered, except `clipboardCopyWithSyntaxHighlightingAction`, which is documented as not ported. The audit 3 fixes re-checked (§A.4, §A.17) hold.

### Decoration tracking and linked editing (#402)

#### A.1 Decoration edges inside a replaced span move to the ends of the new text (P1)
- **Monaco:** `nodeAcceptEdit` (`common/model/intervalTree.ts:416-490`) leaves an edge where it is when it falls within the first `min(deleted, inserted)` characters of a replace (`:456-464`: `ForceStay` when deleting more than inserting, otherwise stickiness decides). Only edges after that common prefix move to `start + inserted` (`:466-476`). An edge exactly at the start of a non-empty replace stays (`ForceStay`, `:447`). `collapseOnReplaceEdit` is at `:437-444`.
- **Port:** `DecorationSet::apply_patch` (`decorations.rs:802-826`) moves every edge with `Patch::transform_offset` (`crates/text/src/patch.rs:73-87`), which sends an offset inside an edit's old range to `new.start` (left bias) or `new.end` (right bias), and an offset at `old.start` with right bias to `new.end`. Cursors use a faithful `adjust_marker` (`cursor/edit.rs:599-645`); model decorations don't. That covers linked-editing ranges, snippet placeholders, find matches, user decorations, the selection anchor and the auto-closed ranges (moved by the same `transform_offset`, `editor.rs:2907-2913`). `collapseOnReplaceEdit` isn't modelled.
- **Repro:**
  - `cargo test --test linked_editing -- --ignored` runs `selection_insert_across_boundary` (`crates/gpui-monaco/tests/linked_editing.rs:457-462`). On `<ooo></ooo>` with `<o` selected, typing `i` should give `ioo></oo>`; it gives `ioo></ioo>`, because the reference range's start inside the replaced `<o` snaps to 0. The test fails with exactly that output.
  - `foo bar baz`, a background decoration on `bar` (default `AlwaysGrows`), select the line, Transform to Uppercase (one equal-length 0..11 replace, `cursor/transforms.rs:950-997`). Monaco keeps the decoration on `BAR`; the port stretches it over the line. With `NeverGrows` it collapses to empty.
- **Fix:** port `nodeAcceptEdit` into `DecorationSet::apply_patch`, applying each edit's old range, deleted and inserted lengths bottom-up as `adjust_marker` does. Add `collapse_on_replace_edit` and un-ignore the test.

#### A.2 Linked editing has no default word pattern, so a space never ends it (P2)
- **Monaco:** the language word pattern is `getLanguageConfiguration(languageId).getWordDefinition()` (`contrib/linkedEditing/browser/linkedEditing.ts:142`), which is the language's `wordPattern` or `DEFAULT_WORD_REGEXP` (`common/languages/languageConfigurationRegistry.ts:399-400`, `common/core/wordHelper.ts:52`). `_currentWordPattern = response?.wordPattern || this._languageWordPattern` (`linkedEditing.ts:334`), so `_syncRanges` always checks the text and ends linked editing on a separator (`:203-210`).
- **Port:** `language_pattern` is `language.config.word_pattern.clone()`, `None` for a language without one (`linked_editing.rs:279-281`, `:359`). With `None`, `sync_linked_ranges` skips the check (`:186-201`) and mirrors anything. The ported tests register a language with a `wordPattern` (`crates/gpui-monaco/tests/linked_editing.rs:262-266`).
- **Repro:** a language with no `word_pattern` and a provider returning both tag-name ranges without one (an XML server, say). In `<foo></foo>`, caret after `foo`, type ` a`. Monaco: `<foo a></foo>` and linked editing ends. Port: `<foo a></foo a>`, and it keeps mirroring.
- **Fix:** fall back to the default word regex (`DEFAULT_WORD_PATTERN`, `cursor/in_place_replace.rs:25`).

#### A.3 Linked ranges ignore option, language and provider changes (P3)
- **Monaco:** `reinitialize` runs on `linkedEditing` configuration changes, provider registry changes and language changes (`linkedEditing.ts:109-119`); it clears the ranges when disabled and requests them at once when enabled (`:121-176`).
- **Port:** `update_options` and language or provider changes don't touch `self.linked`. `linked_editing_cursor_changed` (`linked_editing.rs:112-116`) only schedules while enabled, and stale ranges are cleared by the next edit (`:129-134`). Turning the option on requests nothing until the cursor moves.
- **Repro:** caret in `<div>` with the ranges highlighted, then `update_options` with `linked_editing = false`. Monaco removes the highlight at once; the port keeps it through cursor moves until the next edit.
- **Fix:** on a `linked_editing` option change, language change or provider registration, `clear_linked_editing`, then `update_linked_editing(false)` if enabled.

### Input method edits and undo (#422)

#### A.4 IME replacements over a selection or around a single caret skip `type`/`compositionType` (P2)
- **Monaco:** `NativeEditContext._emitTypeEvent` (`browser/controller/editContext/native/nativeEditContext.ts:434-469`) measures the replacement against the primary selection. A replacement of exactly that selection gives zero deltas, and `_onType` (`:471-476`) calls `viewController.type(text)`: every selection, with interceptors (auto-close, auto-surround) through `Cursor.type` (`common/cursor/cursor.ts:564-587`). A replacement around the caret goes to `compositionType` (`cursor.ts:589`), a `TypingOther` edit that joins the preceding typing's undo group (`common/cursor/cursorTypeEditOperations.ts:799-805`).
- **Port:** `replace_text_in_range` (`editor.rs:6845-6896`) only reaches every cursor through `composition_type_at_cursors` with several cursors, and that returns `None` when the primary selection isn't empty (`editor.rs:2802-2808`). Every other explicit range goes to `replace_range` (`editor.rs:6773-6798`): one edit, collapse to one cursor, kind `Other`, undo stops before and after, `prev_edit_kind` reset.
- **Repro:**
  - Three cursors each selecting `foo`; insert 😀 from the Character Viewer (Ctrl+Cmd+Space). Monaco replaces all three and keeps three cursors; the port replaces the primary one and drops the others.
  - One cursor selecting `foo`; insert `(` from the Character Viewer. Monaco: `(foo)`. Port: `(`.
  - Type `cafe`, press and hold `e`, pick `é`. One Undo removes `café` in Monaco; in the port it turns `é` back into `e`.
- **Fix:** when the range equals the primary selection, run `type_text` at every selection; route single-cursor replacements around the caret through `composition_type`.

#### A.5 `TextModel::undo`/`redo` from outside the editor don't restore the focused editor's selections (P3)
- **Monaco:** `Cursor.onModelContentChanged` restores `e.resultingSelection` in the focused editor whoever called `model.undo()` (`common/cursor/cursor.ts:264-268`); other editors recover from markers.
- **Port:** selections are restored only when the editor is the edit's source: `restored = own && …` (`editor.rs:2915-2921`). `TextModel::undo(None, cx)` (`model.rs:1817`) always recovers from markers.
- **Repro:** type `foo bar`, select `bar`, delete it, keep focus, call `model.update(cx, |m, cx| m.undo(None, cx))`. Monaco selects `bar`; the port leaves a collapsed cursor.
- **Fix:** restore when the edit is an undo or redo and the editor is focused or the source; recover from markers otherwise.

## B. View rendering

Scope: view lines and line rendering, cursors, selections, the current-line highlight, the decorations overlay, guides, rulers, the gutter, scrollbars and the overview ruler, the minimap, view zones, wrapping, padding and zoom. Read from both sides; nothing was built or run.

Checked and matching: whitespace marks (boundary, selection and trailing rules, geometry), control-character pictures and `[U+XXXX]`, tab stops (§B.1 of audit 3), the overflow chip, ligatures, monospace wrapping (indent cap, `bounded`, `wordWrapColumn`, the wrapped content width, guides on continuation lines), horizontal reveal, view zones (ordinal, `afterColumn`, `showInHiddenAreas`, heights), padding, the zoom clamp and font multiplier, cursor geometry, colors, blink timings and smooth caret, selection corners and insets, current-line highlight modes, the audit 3 §B.4–§B.8 overlay fixes, indent and bracket guides, rulers, bracket matching, layout info, line-number modes, glyph lanes, folding controls, overview ruler lanes and cursor marks, scrollbars and the scroll shadow, and the minimap (layout, renderers, decorations, section headers, slider, autohide from audit 3 §D.31, section headers following edits from §D.32).

### Decorations overlay (#410)

#### B.1 Decorations at a wrap point leave a stray half-character box or squiggle on the neighbouring visual line (P2)
Follow-up to audit 3 §B.4/§B.5.
- **Monaco:** decoration ranges map to view lines with start affinity Right and end affinity Left (`common/viewModel/viewModelLines.ts:903-911`, from `common/viewModel/inlineDecorations.ts:185`). A range starting at a wrap point has no piece on the earlier visual line, one ending there none on the later line, and an empty range there goes only to the start of the later line.
- **Port:** `decoration_x_range` (`element.rs:7370-7400`) works on model columns: a zero-width `(width, width)` piece on the earlier segment when `start.column == columns.end`, an `(x0, x0)` piece on the later one when `end.column == columns.start`, and both for an empty range at the wrap column. With `show_if_collapsed` (find matches, unicode highlights, markers), `widen_collapsed` (`element.rs:1378-1401`, called at `:4664`) widens them into half-character boxes; marker squiggles are widened unconditionally (`element.rs:4744-4756`). `segment_intersects` (`element.rs:1808-1816`) already handles this edge for glyphs.
- **Repro:** `word_wrap: on`, a long prose line where a word starts a continuation line. Search for it: Monaco boxes the word; the port also draws a half box at the end of the line above. A diagnostic starting there gets a stray squiggle above. A regex `\b` match at the wrap point shows two boxes.
- **Fix:** on a segment that isn't the row's last, skip the piece when `start.column >= columns.end`; on a later segment skip it when `end.column <= columns.start`; put an empty range only on the later segment.

#### B.2 Touching decorations with the same class aren't merged (P2)
- **Monaco:** `_renderNormalDecorations` sorts by zIndex, className and range, then merges consecutive decorations with the same className and `showIfCollapsed` that intersect or touch (`browser/viewParts/decorations/decorations.ts:164-167`, `common/core/range.ts:409-422`); collapsed-range widening applies to the merged range (`decorations.ts:196-204`).
- **Port:** each decoration paints its own boxes with its own 1px border (`element.rs:4656-4685`, painted at `:6148-6157`); no merge.
- **Repro:** vs-dark, cursor between `()` in `foo()`: Monaco draws one `editorBracketMatch.border` box, the port two with a 2px double border between (`bracket_matching.rs:142-155`). Adjacent `ñé` with `nonBasicASCII` shows the same (one decoration per character, `unicode_highlighter.rs:217-224`). A regex `.` search in an HC theme boxes each character instead of each run.
- **Fix:** before computing pieces, merge consecutive non-whole-line decorations whose ranges touch and whose options and `show_if_collapsed` are equal.

#### B.3 Selection highlights don't fade when the editor loses focus (P3)
- **Monaco:** the full `editor.selectionHighlightBackground` and border apply only under `.focused` (`contrib/wordHighlighter/browser/highlightDecorations.css:6-10`); unfocused, the theming participant at the end of `highlightDecorations.ts` uses `selectionHighlight.transparent(0.5)` with no border. The highlights stay on blur (`contrib/multicursor/browser/multicursor.ts:879-940`).
- **Port:** full color and border regardless of focus (`occurrences.rs:553-555`).
- **Repro:** select `foo` so other occurrences highlight, then focus another input. Monaco halves the opacity and drops the border; the port doesn't change.
- **Fix:** when unfocused, paint selection highlights at half alpha with no border.

#### B.4 High-contrast find and occurrence borders: width and dotted style (P3)
- **Monaco:** `.currentFindMatch` has a 2px `editor.findMatchBorder` (`contrib/find/browser/findWidget.css:232-237`); `.findMatch` borders are dotted in HC (`findWidget.ts:1477`); word and selection highlight borders are dotted in hc-black and hc-light (`highlightDecorations.css:11-41`). The HC cursor's side borders depend on the primary selection only (`browser/viewParts/viewCursors/viewCursors.ts`, `onCursorStateChanged`).
- **Port:** every decoration border is 1px (`element.rs:4681-4683`); the find options (`find.rs:290-336`) and occurrence options (`occurrences.rs:139-155`, `:553-568`) set no `border_style`. The HC cursor checks whether any selection is non-empty (`element.rs:7151`).
- **Repro:** HC Dark, search: the current match has a solid 1px border (Monaco 2px); other matches and word highlights are solid (Monaco dotted).
- **Fix:** add a border width to `DecorationOptions` (2px for the current match); use `BorderStyle::Dashed` for these in HC themes (GPUI has no dotted style); check only the primary selection for the cursor.

### View lines and cursors (#426)

#### B.5 Changing `read_only` or `cursor_blinking` at runtime doesn't reset the blink (P3)
- **Monaco:** `onConfigurationChanged` calls `_updateBlinking()` (`browser/viewParts/viewCursors/viewCursors.ts:111-127`), so a read-only or `solid` cursor turns solid at once.
- **Port:** `restart_blink` runs only on focus (`editor.rs:978`) and cursor changes (`editor.rs:3252`); `update_options` (`editor.rs:1474-1545`) doesn't call it, and the blink task (`editor.rs:3385-3402`) keeps toggling.
- **Repro:** focused editor, default blinking, `update_options` with `read_only: true`. The port keeps blinking (or stays hidden) until the next cursor move.
- **Fix:** call `restart_blink` from `update_options` when `read_only` or `cursor_blinking` changes.

#### B.6 The block cursor covers one code point, not one grapheme (P3)
- **Monaco:** `_getGraphemeAwarePosition` steps by grapheme with `strings.getCharContainingOffset` (`base/common/strings.ts:641-658`); the block's width and text cover the grapheme (`browser/viewParts/viewCursors/viewCursor.ts:157-162`, `:213-232`).
- **Port:** the block covers one UTF-16 unit, or a surrogate pair (`element.rs:5161-5175`); only that range is redrawn in the cursor text color (`:5232-5239`).
- **Repro:** decomposed `é` then `x`, block cursor at column 0. Monaco shows `é` in the cursor text color; the port hides the accent. ZWJ emoji and flags get a narrow block.
- **Fix:** extend the block to the next grapheme boundary for width and glyph range.

#### B.7 The active bracket-pair guide is wrong when the primary cursor is off screen (P3)
- **Monaco:** with the cursor outside the viewport, `getLinesBracketGuides` queries the pairs at the cursor; the active pair is the innermost pair containing it, visible or not (`common/model/guidesTextModelPart.ts:296-310`).
- **Port:** `lines_bracket_guides` picks the active pair with `rposition` over the visible pairs only (`view/guides.rs:344-348`, from `bracket_pairs_in_rows`, `bracket_colors.rs:203-250`).
- **Repro:** `bracketPairs: true`, cursor in an inner `{}` near the end of a long outer `{}`, scroll so only the outer block's top shows. Monaco highlights no guide; the port marks the outer one active, horizontal guide included.
- **Fix:** find the active pair among all pairs containing the cursor; highlight it only if visible.

#### B.8 Advanced wrapping uses the monospace continuation-indent formula (P3)
- **Monaco:** the DOM line-break computer indents continuation lines by the existing indent plus `tabSize × N`, without rounding to a tab stop, and its stick-to-column-1 check looks only at the existing indent (`browser/view/domLineBreaksComputer.ts:69-71`, `:96-128`, `:179`).
- **Port:** always `wrapped_text_indent_length` (`view/line_breaks.rs:157-191`, called at `:367`), which rounds to the next tab stop and has the monospace cap.
- **Repro:** `wrapping_strategy: Advanced`, `wrapping_indent: Indent`, `tab_size: 4`, 2 leading spaces: Monaco indents by 6, the port by 4. `DeepIndent` with 1 space: Monaco 9, port 8.
- **Fix:** when `measure` is set, use `existing + tab_size * n` and apply the column-1 check to the existing indent only.

#### B.9 Whitespace marks ignore injected text (P3)
- **Monaco:** `_applyRenderWhitespace` scans the view line, `before`/`after` injected text included (`browser/viewParts/whitespace/whitespace.ts:133-157`).
- **Port:** `whitespace_columns` scans the segment's model text (`element.rs:5468-5486` → `view/guides.rs:200-271`).
- **Repro:** `render_whitespace: Trailing`, `foo  ` with an `after: "← note"` decoration at the end. Monaco draws no dots (the spaces aren't trailing in the view line); the port draws two. With `All`, Monaco dots the space inside the injected text; the port doesn't. Inlay hints are unaffected (their spaces are NBSP).
- **Fix:** scan the view-line text including injected text through the view-column map.

#### B.10 NUL isn't shown as U+FFFD with `renderControlCharacters: false` (P3)
- **Monaco:** a NUL is emitted as `&#00;` (`common/viewLayout/viewLineRenderer.ts:1176-1183`), which HTML turns into U+FFFD.
- **Port:** `substitute` returns `None` for `\0` when the option is off (`element.rs:420-428`), so the raw NUL reaches the shaper.
- **Repro:** `a\0b`, `render_control_characters: false`. Monaco `a�b`; the port shows whatever the font draws.
- **Fix:** map `'\0'` to `Substitute::Replacement` when the option is off.

### Font metrics (#420)

#### B.11 The typical half-width width is measured from `m`, not `n` (P3)
- **Monaco:** `typicalHalfwidthCharacterWidth` is the width of `'n'` (`browser/config/fontMeasurements.ts:183`, `:249`). It drives the viewport and wrapping column (`common/config/editorOptions.ts:3019`), rulers (`browser/viewParts/rulers/rulers.ts:34`) and `scrollBeyondLastColumn` (`common/viewLayout/viewLayout.ts`).
- **Port:** `em_width = width("m")` (`element.rs:244`) feeds the layout input (`:3199`), rulers (`:5434`), the content width (`:3424`) and `font_info()` (`api.rs:2216`).
- **Repro:** `font_family` Helvetica, `rulers: [80]`. Monaco places the ruler at 80 × width(`n`); the port about 50% further right, and wraps at fewer columns. Monospace fonts are unaffected.
- **Fix:** measure `"n"` for the typical half-width width.

### Gutter and minimap (#427, #431)

#### B.12 The minimap fold highlight is only partly fixed (P3)
Incomplete fix of audit 3 §D.13.
- **Monaco:** the fold decoration is whole-line (`contrib/folding/browser/foldingDecorations.ts:41-48`), so its view range runs from column 1 of the header to the line end (`common/viewModel/inlineDecorations.ts:178-181`). The minimap draws a 50% band on every view line of the header (`browser/viewParts/minimap/minimap.ts:1623-1670`) and a full-strength inline mark over the header's text (`:1699-1735`, `:1747-1771`).
- **Port:** the range is one point at the end of the header line (`minimap.rs:796-830`), so `decoration_on_line`'s `x2 > x1` check skips the inline mark (`minimap.rs:687-689`), and the band is drawn only on the last wrapped piece.
- **Repro:** fold a function with the minimap on: Monaco shows the band plus the fold color over the header's characters; the port only the band. A wrapped header is shaded on all pieces in Monaco, the last in the port.
- **Fix:** convert `(header, 0)` with `to_view(.., false)` through `(header, line end)` with `to_view(.., true)`.

#### B.13 Line decorations in the gutter use model rows, not the decoration's view range (P3)
- **Monaco:** `LinesDecorationsOverlay` and `MarginViewLineDecorationsOverlay` work on view ranges (`browser/viewParts/linesDecorations/linesDecorations.ts:~75-88`): `firstLineDecorationClassName` only on the range's start line, others only on the view lines the range touches, and the `.cldr` element is `decorationsWidth` wide, including the 16px folding column.
- **Port:** draws on every laid-out line whose model row is in the range (`element.rs:4641-4644`); `first_line_decoration` checks only `columns.start == 0`, so it shows on the first segment of every row (`element.rs:4757-4767`); bars, text and margin backgrounds repeat on every wrapped segment; the tooltip hit box leaves out the folding column (`element.rs:4778-4789`, `:4612`).
- **Repro:** `first_line_decoration` on rows 2–5: Monaco one line, the port four. A `gutter_bar` on columns 0–5 of a line wrapping into 3 pieces: Monaco first piece, port all three. Hovering the folding-column part: Monaco shows the tooltip, the port doesn't.
- **Fix:** use `segment_intersects`, limit `first_line` to the start view line, widen the tooltip box to `decorations_width`.

#### B.14 Glyph-margin codicons use the editor font size (P3)
Upstream CSS, not vendored.
- **Monaco:** a glyph is a `cgmr codicon` element centered in its lane (`browser/viewParts/glyphMargin/glyphMargin.ts:425-433`), sized by `codicon.css` (`font: 16px/1 codicon`, upstream).
- **Port:** shapes the glyph at `metrics.font_size` (`element.rs:5762-5774`).
- **Repro:** a breakpoint codicon with the default 14px font: the port draws 14px, and it follows the editor font.
- **Fix:** shape codicon glyphs at 16px, centered in the lane, as `paint_icon` does.

#### B.15 With `lineNumbers: interval`, the always-numbered last line differs when lines are folded (P3)
A Monaco quirk; decide and document (#431).
- **Monaco:** always numbers the line whose model number equals `viewModel.getLineCount()`, the *view* line count (`browser/viewParts/lineNumbers/lineNumbers.ts:137-140`).
- **Port:** uses the model line count (`element.rs:5557-5560`).
- **Repro:** 25 lines, interval mode, lines 2–10 folded (16 view lines). Monaco numbers line 16 and leaves 25 blank; the port numbers 25.
- **Fix:** copy the quirk, or list the difference in `book/src/differences.md`.

## C. Widgets and menus

Scope: content and glyph hovers, markdown, suggest and parameter hints, find and find options, quick input, code actions and the lightbulb, rename, the context menu, color pickers, the read-only message, code lenses, inlay hints, peek, the marker zone, the inline toolbar and gutter menu, paste and drop widgets and inline progress. §C.1 and §C.5 were confirmed in the playground; the rest were read from both sides. No findings in the rename box, the context menu, the read-only message or inlay hint rendering. The audit 3 §C fixes hold except §C.16 (see §C.12).

### Focused hover and widget keys (#400)

#### C.1 Keys in a focused hover fall through to the editor and edit the document (P1)
- **Monaco:** a focused hover is a DOM subtree outside the textarea, and the editor's typing and editing keybindings are `editorTextFocus`-gated (`browser/coreCommands.ts`, e.g. `tab`/`outdent` at `:2004-2035`), so Enter, Backspace, Delete, Cmd+V and Cmd+X in the hover never reach the model. Only the hover's `hoverFocused` bindings act (`contrib/hover/browser/hoverActions.ts:182-413`).
- **Port:** the hover is a deferred draw under the editor's `code-editor` key context (`editor.rs:7109-7112`), and GPUI matches a binding at any context depth (`depth_of`), so `enter → Newline`, `backspace → DeleteLeft`, `delete`, `secondary-v` (`actions.rs:1762-1768`) match with focus in the hover. The hover binds only scroll and Escape keys (`actions.rs:2243-2258`) and handles only `Tab`/`Outdent` (`hover.rs:671-678`); the bubbling `Newline` reaches `CodeEditor::newline` (`editor.rs:4245`, registered at `:7462`), which has no focus check. `widget_key_context`'s doc says the editor's bindings "don't match in the widget" (`context_keys.rs:648-653`), but it only drops `KEY_CONTEXT` from the widget's own context, not its ancestors'.
- **Repro (playground):** `PLAYGROUND_KEYS="down right×13 cmd-k cmd-i wait×4 cmd-k cmd-i wait wait enter"` on `let e = Edit::new();`. The hover opens and takes focus, and Enter splits the line into `let e = E` / `dit::new();` while the hover closes. Monaco leaves the text alone. Backspace, Delete and Cmd+V do the same. Other focusable widgets that don't bind these keys likely leak too (audit 3 §C.35 fixed only Tab/Shift+Tab).
- **Fix:** have the hover and every focusable widget root swallow the editor's editing actions (or stop propagation of unhandled key-bound actions), or gate the editing handlers on `focus_handle.is_focused(window)`, Monaco's `editorTextFocus`.

#### C.2 Hover copy buttons, status bar actions and links can't be used from the keyboard (P2)
- **Monaco:** the copy button is a `SimpleButton` (`contrib/hover/browser/hoverCopyButton.ts:31-38`): `tabIndex = 0`, `role="button"`, Enter/Space run it (`contrib/find/browser/findWidget.ts:1418-1438`), and it triggers on click. Status bar actions are `HoverAction`s (`contentHoverStatusBar.ts:49`; `base/browser/ui/hover/hoverWidget.ts`, upstream, not vendored: `tabIndex 0`, Enter/Space), and markdown links are focusable anchors.
- **Port:** the copy button is a div with only `on_mouse_down` (`hover.rs:729-764`); status bar entries also take only `on_mouse_down` (`hover.rs:~1217`); Tab moves only between rows (`hover.rs:388-416`), and the `HoverWidget` context has no Enter/Space binding (`actions.rs:2243-2258`). `docs/MONACO_PARITY.md:196` notes they "aren't tab stops of their own" without a reason. Enter falls through to the editor (§C.1).
- **Repro:** hover a diagnostic, Cmd+K Cmd+I twice, Tab to the marker row, Enter or Space. Monaco (after Tab onto the button) copies and announces "Copied to clipboard"; the port copies nothing. Same for "Quick Fix..." and "View Problem".
- **Fix:** give the copy button and status bar actions focus handles in the Tab order (or bind Enter/Space on a focused row to its first action), and run them on click.

#### C.3 Tab on a focused find-widget control leaves the widget in read-only editors and tab focus mode (P2)
- **Monaco:** buttons and toggles are DOM tab stops (`contrib/find/browser/findWidget.ts:1136-1160`, `:1202-1237`); the editor's `tab`/`outdent` need `editorTextFocus`, so read-only and tab focus mode don't change the cycle.
- **Port:** on a focused control, the outer editor's `tab → FocusNextElement` under `TAB_MOVES_FOCUS` (`actions.rs:1861`) outranks `tab → Tab` (`actions.rs:2390-2404`); `FindWidget` handles only `Tab`/`Outdent` (`find_widget.rs:1808-1809`), so the action bubbles to `window.focus_next` (`editor.rs:7811`). The hover handles this case (`hover.rs:679-684`).
- **Repro:** `read_only = true`, Cmd/Ctrl+F, Tab (Match Case), Tab. Monaco goes to Match Whole Word; the port leaves the widget.
- **Fix:** handle `FocusNextElement`/`FocusPreviousElement` on the `FindWidget` root through `move_focus`.

### Content and glyph hover (#405)

#### C.4 The hover waits for every provider and never shows "Loading..." (P2)
- **Monaco:** `HoverOperation` shows the sync parts (markers, decoration messages) at the hover delay, streams each async result as it arrives, and adds a "Loading..." part at 3× the delay while providers are pending (`contrib/hover/browser/hoverOperation.ts:55-62`, `:106-108`, `:115-178`; `markdownHoverParticipant.ts:96-98`; `contentHoverWidgetWrapper.ts:74`, `:184-194`).
- **Port:** `schedule_text_hover` awaits the color picker, then every provider in turn, before showing anything (`hover.rs:2148-2203`); there's no loading part.
- **Repro:** a marker on a word and a hover provider taking 2s. Monaco shows the diagnostic at 300ms, "Loading..." at 900ms, then the provider's part; the port shows nothing for 2s.
- **Fix:** show sync parts after the delay, append provider parts as each resolves, and add a "Loading..." part after `3 * delay` until done.

#### C.5 Hovers start while the context menu is open (P2)
- **Monaco:** `onDidShowContextMenu` hides the hover and sets `_ignoreMouseEvents` until the menu hides; mouse moves, downs, leaves, scrolls and keys are ignored meanwhile (`contrib/hover/browser/contentHoverController.ts:73-80`, `:120-122`, `:137-139`).
- **Port:** `show_context_menu`/`show_menu` (`context_menu.rs:751-778`, `:910+`) don't touch the hover, and mouse moves skip the hover only over widget bounds (`editor.rs:6052-6075`).
- **Repro (playground):** Shift+F10, move the mouse off the menu onto a word with a hover (`Edit`), wait 300ms. The port opens the hover beside the menu; Monaco doesn't.
- **Fix:** hide the hover when a context menu shows, and skip `hover_mouse_move` while it's visible.

#### C.6 A visible glyph hover isn't recomputed when decorations change (P2)
- **Monaco:** `GlyphHoverWidget` listens to `onDidChangeModelDecorations` and, while visible, restarts its operation for the same line and lane (`contrib/hover/browser/glyphHoverWidget.ts:51`, `:87-94`); an empty result hides it (`:132-140`).
- **Port:** `schedule_gutter_hover` computes the messages once (`hover.rs:2282-2312`); the decorations-changed path only re-emits and repaints (`editor.rs:3053-3056`).
- **Repro:** hover a glyph with a `glyph_margin_hover_message`, then remove or change the decoration from a timer with `delta_decorations`. Monaco updates or hides the card; the port keeps the old message until the mouse moves to another row.
- **Fix:** on `TextModelEvent::DecorationsChanged`, if the glyph card is visible, recompute `glyph_hover_messages` (delayed like Monaco) and update or hide.

#### C.7 Moving between glyph rows waits for the hiding delay first (P3)
- **Monaco:** the glyph hover has no grace period: a new line or lane hides the card and starts the delayed operation at once (`glyphHoverController.ts` `_onEditorMouseMove`/`_reactToEditorMouseMove`; `glyphHoverWidget.ts:110-121`). Only the content hover reschedules after `hidingDelay` (`contentHoverController.ts:236-243`).
- **Port:** `hover_mouse_move` applies the `hiding_delay` grace to any visible hover, glyph ones included (`hover.rs:2035-2055`).
- **Repro:** adjacent lines with glyph hover messages; hover one, move to the next. Monaco swaps after ~300ms, the port after ~600ms with the stale card in between.
- **Fix:** skip the grace period when `hover.glyph` is set.

#### C.8 Hover row separators and rules use the full border color (P3)
- **Monaco:** `.hover-row:not(:first-child):not(:empty)` and `hr` use `editorHoverWidget.border` at 50% alpha (`contrib/hover/browser/hoverContribution.ts:43-49`).
- **Port:** rows use `colors.hover_border` at full alpha (`hover.rs:560-562`, the verbosity column `:925`); markdown `hr` uses `textSeparator.foreground` (`hover.rs:1429`, `markdown.rs:1159`).
- **Repro:** a hover with a marker and a provider part in a theme with a strong `editorHoverWidget.border`: the separator is twice as opaque.
- **Fix:** `hover_border.opacity(0.5)` for row borders and the hover's markdown separator.

### Markdown and links (#415)

#### C.9 Markdown links to models aren't opened in an editor when no link opener takes them (P2)
- **Monaco:** `OpenerService` falls back to `EditorOpener` (`browser/services/openerService.ts:73-102`, `:146-148`), which calls `ICodeEditorService.openCodeEditor` with the focused editor: `registerEditorOpener` handlers first (`standalone/browser/standaloneEditor.ts:478-499`), then the standalone service, which for the editor's own model selects the link fragment's position (`#L10,5`, `extractSelection`, upstream, not vendored) and reveals it centered (`standalone/browser/standaloneCodeEditorService.ts:63-91`).
- **Port:** `markdown.rs` `open_link` (`:820-836`) offers the url to link openers, then opens only http(s)/mailto (`is_openable`, `:761-766`) and trusted `command:` links; anything else is dropped. Editor openers (`openers.rs:61-79`) are never asked.
- **Repro:** a hover returning ``[see line 10](file:///path/to/this/model.rs#L10)`` for a model with that uri, no link opener; click it. Monaco moves to line 10 and centers it; the port does nothing. A link to another registered model doesn't reach `register_editor_opener` either.
- **Fix:** after the link openers, parse the uri and `#L<line>[,<col>]`; for the editor's model, select and `reveal_range_in_center`, otherwise route through `open_location_with_openers`.

#### C.10 Markdown rendering gaps: unlabeled code blocks, `<kbd>`, task lists (P3)
- **Monaco:** code blocks without a language use the editor's model language (`browser/widget/markdownRenderer/browser/editorMarkdownCodeBlockRenderer.ts:36-46`; hovers pass `context: editor`, `markdownHoverParticipant.ts:525-527`; parameter hints too, `parameterHintsWidget.ts:271-272`). `<kbd>` is a keycap (`renderedMarkdown.css:6-17`). GFM task lists render checkboxes without bullets (`renderedMarkdown.css:19-21`).
- **Port:** unlabeled blocks get no highlighting (`markdown.rs:402-412`, `:1074-1081`); `kbd` is styled like `code` (`markdown.rs:532`); task lists aren't enabled (`markdown.rs:743-746`), so `- [ ] item` shows a bullet and a literal `[ ]`.
- **Repro:** a Rust hover with an unlabeled `fn x() {}` block, `<kbd>Ctrl</kbd>` (with `support_html`) and `- [x] done`.
- **Fix:** pass the editor's language id as the default code language, style `kbd` with `keybindingLabel.*`, enable `Options::ENABLE_TASKLISTS` and render a disabled checkbox.

### Suggest widget and parameter hints (#414, #423)

#### C.11 The suggest list is never pushed above multi-line inline-completion ghost text (P2)
- **Monaco:** `InlineCompletionsController` calls `forceRenderingAbove()` when the inline completion extending the suggest selection has 2+ lines of ghost text, and `stopForceRenderingAbove()` otherwise (`contrib/inlineCompletions/browser/controller/inlineCompletionsController.ts:298-307`); `SuggestWidget._layout` then goes above with more than 150px of room (`contrib/suggest/browser/suggestWidget.ts:894-897`, `:1014-1023`; `suggestController.ts:215-217`, `:747-760`).
- **Port:** `layout_list` picks a side from fit and room only (`suggest_widget.rs:472-476`); nothing in `inline_completions/`, `suggest_inline_completions.rs` or `completion.rs` asks for above.
- **Repro:** an inline provider extending `foo` with `foo(\n  a,\n  b\n)`; type `fo` near the top. Monaco moves the list above; the port's list covers the ghost lines.
- **Fix:** a `force_above` flag on `SuggestWidget`, set from the inline completion state and honored in `layout_list` when `room.above > 150px`.

#### C.12 The `suggestionCanResolve` and `suggestionMakesTextEdit` context keys differ from Monaco (P3)
Incomplete fix of audit 3 §C.16.
- **Monaco:** `CanResolve` = `!!resolveCompletionItem || !!documentation || detail !== label` (`contrib/suggest/browser/suggestController.ts:212`); `MakesTextEdit` is false only with `acceptSuggestionOnEnter: smart`, Auto state, no additional edits, no snippet and typed text equal to `insertText` (`suggestController.ts:189-206`).
- **Port:** `suggestionCanResolve` is `item.data.is_some()` (`context_keys.rs:428`); the widget's `focused_can_resolve` was fixed (`completion.rs:368-377`) but not the key. `suggestionMakesTextEdit` only compares the insert text with the text before the cursor (`context_keys.rs:407-416`), unlike the port's own `suggestion_makes_text_edit` (`completion.rs:2263-2290`).
- **Repro:** word-based suggestions: a `when` on `suggestionCanResolve` is true in Monaco, false in the port. With `accept_suggestion_on_enter: On`, type a snippet item's full text: Monaco's `suggestionMakesTextEdit` is true, the port's false.
- **Fix:** set the keys from `focused_can_resolve()` and `suggestion_makes_text_edit()`.

#### C.13 Down/Up/PageUp/PageDown move the caret while "Loading..." or "No suggestions." shows (P3)
- **Monaco:** the select commands need `Visible && (MultipleSuggestions || !HasFocusedSuggestion)`, and Visible is set in the Loading and Empty states (`suggestWidget.ts:486-505`, `:530`), so the key is consumed (`suggestWidget.ts:624-634`; `suggestController.ts:922-935`).
- **Port:** `suggest_navigate` returns false with no filtered items (`completion.rs:2120-2126`), so the caret moves (`editor.rs:3520-3531`) and the request is cancelled.
- **Repro:** Ctrl+Space where nothing completes, Down while "No suggestions." shows. Monaco's caret stays; the port's moves.
- **Fix:** return true while the widget shows a message (and while loading for explicit requests).

#### C.14 Suggest status bar: "Replace" for items without a distinct replace range; hard-coded key labels (P3)
- **Monaco:** "Insert" whenever `!suggestionHasInsertAndReplaceRange`, mode-dependent only with both ranges (`suggestController.ts:858-876`); labels from the keybinding service (`suggestWidgetStatus.ts:40-47`).
- **Port:** the label follows `insert_mode` only (`suggest_widget.rs:1573-1591`); literal "⏎"/"Enter", "⇧⏎", "⌃Space", "↓" (`suggest_widget.rs:1583`, `:1589`, `:1601`, `:1613`) instead of `keybinding_label::command_label` (`keybinding_label.rs:334`).
- **Repro:** `suggest.insert_mode: Replace`, items without a replace range: Monaco "Insert (⏎)", port "Replace (⏎)". Rebinding `ToggleSuggestionDetails` still shows "⌃Space".
- **Fix:** "Insert" when the ranges don't differ; key text from `command_label`.

#### C.15 High-contrast themes keep 1px borders on suggest, details and parameter hints (P3)
- **Monaco:** 2px in hc-black and hc-light (`contrib/suggest/browser/media/suggest.css:32-37`; `suggestWidgetDetails.ts:116-127` feeds it into layout; `contrib/parameterHints/browser/parameterHints.css:20-23`).
- **Port:** `const BORDER: f32 = 1.` (`suggest_widget.rs:46`), `.border_1()` (`suggest_widget.rs:1943`, `:2268`; `signature_help.rs:304`); `theme.high_contrast` (`theme.rs:1618`) isn't used there.
- **Repro:** hc-black, Ctrl+Space and Cmd/Ctrl+Shift+Space.
- **Fix:** take the border width from `theme.high_contrast` in rendering, `layout_list` and `place_details`.

#### C.16 Suggest row text: `.details-label` dimmed, deprecated dimming too wide, newlines not stripped (P3)
- **Monaco:** `.details-label` has no opacity (`suggest.css:191-197`, `:217-223`); `.deprecated { opacity: 0.66 }` covers only the icon label, not `.signature-label` (`suggestWidgetRenderer.ts:122-126`; `suggest.css:318-325`); `stripNewLines` on `detail`, `label.detail` and `label.description` (`suggestWidgetRenderer.ts:211-219`, `:252-254`).
- **Port:** the unfocused right label is at 0.7 (`suggest_widget.rs:1534`); deprecated opacity covers the whole left div (`:1501-1519`); `label_detail`/`label_description` are raw (`:1392`, `:1517`), and `detail` newlines become spaces (`:1397`).
- **Repro:** items with `label.description` are fainter in the port; a deprecated item with `label.detail = "(x)"` gets 0.66×0.6; `label.detail = "(a,\n b)"` clips ` b)` where Monaco shows `(a, b)`.
- **Fix:** drop the 0.7, move the deprecated opacity to the label, strip `\r\n|\r|\n` from all three.

#### C.17 The read-more chevron is hidden while details are open (P3)
- **Monaco:** shown on `.focused:hover` whenever the item can expand, hidden only with `.with-status-bar`/`.docs-side` (`suggest.css:80-83`, `:285-314`; `suggestWidgetRenderer.ts:227-244`); clicking it closes details.
- **Port:** `read_more = is_selected && !show_details && …` (`suggest_widget.rs:1400-1401`).
- **Repro:** status bar off, details open, hover the focused row: Monaco shows the chevron, the port doesn't.
- **Fix:** drop `!show_details`.

#### C.18 Suggest row icon metrics with `showIcons: false` and color swatches (P3)
- **Monaco:** `.no-icons` hides only `.suggest-icon::before`, and the element keeps `margin-right: 4px` (`suggest.css:345-354`); the `customcolor` icon is sized to the line height by `configureFont` (`suggestWidgetRenderer.ts:153-154`).
- **Port:** no icon element when `show_icons` is off (`suggest_widget.rs:1406`); the swatch box is `.w(px(16.))` (`:1422-1424`).
- **Repro:** `suggest.show_icons: false`: labels sit 4px further left. Color items' labels sit ~3px left at the default line height.
- **Fix:** keep a 4px spacer; size the swatch to `item_height`.

#### C.19 The parameter hints body has no scrollbar (P3)
- **Monaco:** the body is a `DomScrollableElement` (`contrib/parameterHints/browser/parameterHintsWidget.ts:89-94`) with the 10px fading scrollbar.
- **Port:** plain `.overflow_y_scroll()` (`signature_help.rs:262`); the hover already uses `crate::scrollbars` (`hover.rs:616-618`).
- **Repro:** a signature with docs taller than the cap: nothing shows there's more.
- **Fix:** reuse `crate::scrollbars`.

#### C.20 The details scroll isn't reset when the focused item re-renders after resolve (P3)
- **Monaco:** `renderItem` always sets `_body.scrollTop = 0` (`suggestWidgetDetails.ts:206`), including after resolve (`suggestWidget.ts:446-447`).
- **Port:** resets only when the focused label or index changes (`suggest_widget.rs:886-891`).
- **Repro:** expand details and scroll while the item resolves; Monaco jumps to the top on resolve, the port keeps the offset.
- **Fix:** also reset when `resolving` goes false for the same item.

### Find widget (#408)

#### C.21 Enter in the find input skips Find Next's handling, so `find.closeOnResult` doesn't work (P2)
- **Monaco:** Enter in the find input runs `NextMatchFindAction` (`contrib/find/browser/findController.ts:786-790`) → `matchFindAction` (`:728-775`): an undo stop, closing with `closeOnResult` (`:748`), and a restart and "no results" announcement on failure.
- **Port:** `FindWidget::on_enter` (`find_widget.rs:873-887`) calls `editor.find_next` directly (`find.rs:1532-1536`); only Shift+Enter goes through `find_match_action` (`find.rs:2095-2140`).
- **Repro:** `find.close_on_result = true`, Cmd/Ctrl+F, type `foo`, Enter. Monaco selects and closes; the port stays open (Shift+Enter closes).
- **Fix:** call `find_match_action(true, …)` in `on_enter`.

#### C.22 Cmd/Ctrl+D and Select All Occurrences never flash the find options widget (P2)
- **Monaco:** a multicursor session using find options calls `findController.highlightFindOptions()` on each step and on select-all (`contrib/multicursor/browser/multicursor.ts:394`, `:445`, `:462`, `:471-474`; always for sessions from an empty cursor, `:315`), showing the options for 2s with find closed (`findController.ts:541-549`).
- **Port:** `multi_cursor.rs` never calls `highlight_find_options`; the only path is `apply_find_options` (`find.rs:600-605`), which needs an existing options widget and a changed toggle.
- **Repro:** `foo foo foo`, cursor in the first, Cmd/Ctrl+D twice. Monaco shows Aa/ab/.* at the top right; the port nothing.
- **Fix:** make `highlight_find_options` `pub(crate)` and call it from the session paths when `uses_find_options` and find is closed.

#### C.23 Closing find destroys the widget: the resized width is lost and there's no slide-out (P3)
- **Monaco:** one `FindWidget` per editor; `_hide` removes `visible` (`findWidget.ts:625-646`) so the 200ms transform slides it up (`findWidget.css:13`), and the sash width survives (`findWidget.ts:1271-1330`).
- **Port:** `close_find` drops the entity (`find.rs:2329-2331`) and `find_start_with` builds a new one (`find.rs:1880-1891`) with the initial `width`/`resized`/`matches_width` (`find_widget.rs:461-464`); it vanishes at once.
- **Repro:** widen to ~600px, Escape, Cmd/Ctrl+F. Monaco reopens at 600px after sliding out; the port at 419px.
- **Fix:** keep the entity across closes and animate the hide.

#### C.24 The find Close button's tooltip has no keybinding (P3)
- **Monaco:** "Close (Escape)" (`findWidget.ts:1139`; bindings `findController.ts:1071-1081`).
- **Port:** looks up `CloseFindWidget` (`find_widget.rs:198`, `:1151-1160`), but nothing binds it (Escape is `RemoveSecondaryCursors`, `actions.rs:1756`), so `command_binding` (`keybinding_label.rs:310-331`) finds nothing: "Close".
- **Repro:** hover the ×.
- **Fix:** take the label from the Escape binding, or bind `escape → CloseFindWidget` in the find context.

#### C.25 Find widget chrome deviates from `findWidget.css` (P3)
- **Monaco:** the toggle-replace button is `.button.left { margin: 4px 0 4px 5px }` over `.button.toggle { left: 0; top: 0; width: 18px }` (`findWidget.ts:1244`, `findWidget.css:151-168`); Close is `top: 5px; right: 4px` (`:277-281`); the shadow is `--vscode-shadow-lg` (`:18`); the find-in-selection toggle has a 5px radius and `toolbar.hoverBackground`, checked or not (`:144-149`, `:255-259`).
- **Port:** toggle `left(0) top(0) bottom(0) w(18)` (`find_widget.rs:1575-1579`); Close `top(4) right(3)` (`:1615-1616`); `widget.shadow` 0 0 8px 2px (`:1728-1736`) while its options widget uses `card::shadow_lg()` (`:1905`); a 3px radius and `inputOption.hoverBackground` only when unchecked (`:1204`, `:1218-1221`).
- **Repro:** Cmd/Ctrl+F; compare the chevron column, the ×, the shadow and the selection toggle's hover.
- **Fix:** left 5/top 4/bottom 4, top 5/right 4, `card::shadow_lg()`, a 5px radius with the toolbar hover.

### Quick input and color pickers (#429)

#### C.26 Go to Symbol ignores `container_name` and doesn't match or highlight containers (P3)
Found by the widgets audit (26) and the API audit (10).
- **Monaco:** `_flattenDocumentSymbols` uses `entry.containerName || parent name` (`contrib/documentSymbols/browser/outlineModel.ts:372-379`). Go to Symbol shows it as the description, scores a second query word against it and sets `highlights.description` (`contrib/quickAccess/browser/gotoSymbolQuickAccess.ts:279`, `:318-319`, `:337-340`).
- **Port:** `flatten_symbols` always uses the parent's name (`outline.rs:84-101`), so `DocumentSymbol::container_name` (`providers.rs:1514`) has no reader; the LSP bridge fills it only for flat `SymbolInformation` results (`crates/gpui-monaco-lsp/src/convert.rs:783`), which have no parent. Container matches are discarded (`quick_access.rs:1375-1381`) and the description is plain text (`:573-575`).
- **Repro:** a flat result `{ name: "run", container_name: "Server" }`, then `@run Server`: Monaco shows "run · Server", matches it and bolds `Server`; the port shows no description and finds nothing. `@foo bar` with `foo` in `Bar` bolds `Bar` in Monaco only.
- **Fix:** prefer `container_name` over the parent's name in `flatten_symbols`; keep container scores and add `description_highlights` on `QuickItem`, rendered with `StyledText`.

#### C.27 Go to Symbol activates and previews the first symbol when the cursor is in none (P3)
- **Monaco:** opens with `itemActivation: NONE` (`standalone/browser/quickAccess/standaloneGotoSymbolQuickAccess.ts:63`) and activates only the innermost enclosing symbol (`gotoSymbolQuickAccess.ts:193-198`).
- **Port:** `goto_symbol_rows` returns `None` (`quick_access.rs:1438-1445`), but `QuickInput::new`/`refresh` fall back to item 0 and preview it (`:276`, `:296`).
- **Repro:** cursor on a blank line before the first function, Cmd/Ctrl+Shift+O: the port scrolls to the first symbol.
- **Fix:** leave `active` as `None` for an empty `@` query without an enclosing symbol.

#### C.28 The zero-based offset toggle is a "0" glyph, not `index-zero` (P3)
- **Monaco:** `Codicon.indexZero` (`contrib/quickAccess/browser/gotoLineQuickAccess.ts:84-90`).
- **Port:** text `"0"` (`quick_access.rs:850-866`), though `Codicon::IndexZero` exists (`codicons.rs:1038`).
- **Repro:** type `::`.
- **Fix:** `codicon(Codicon::IndexZero)`.

#### C.29 Standalone quick-input highlight colors (P3)
Partly upstream, not vendored.
- **Monaco:** `standaloneQuickInput.css:10-33` overrides the highlights: `#0097fb` for vs-dark rows, `#9DDDFF` for focused vs rows.
- **Port:** the registry's `list.highlightForeground`/`quickInputList.focusHighlightForeground` (`quick_access.rs:504-508`; `theme/color_registry.rs:418-419`, `:459`).
- **Repro:** F1, `fo`, in vs-dark and vs.
- **Fix:** apply the standalone overrides for the built-in themes.

#### C.30 The color picker loses the color's range after additional edits or multi-line text (P3)
Found by the widgets audit (36) and the API audit (12).
- **Monaco:** `updateEditorModel` tracks the replace range with a `GrowsOnlyWhenTypingAfter` decoration across `executeEdits` and returns the tracked range (`contrib/colorPicker/browser/colorPickerParticipantUtils.ts:45-63`).
- **Port:** `apply_color_presentation` returns `start..(start.row, start.column + text.len_utf16)`, measured before the edits (`color_picker.rs:768-790`, used by `update_editor` at `:268`); it ignores `additional_text_edits` above the color and newlines in the text.
- **Repro:** a presentation with `additional_text_edits: [((0,0)..(0,0), "use x;\n")]` for a color on row 5; drag the hue twice. Monaco's second write replaces the color, now on row 6; the port's overwrites the stale row-5 range.
- **Fix:** track the range with a decoration (or map it through the edits) and return that.

#### C.31 The standalone color picker doesn't close when the mouse moves onto a color swatch (P3)
- **Monaco:** hides on mouse move over a `colorpicker-color-decoration` (`contrib/colorPicker/browser/standaloneColorPicker/standaloneColorPickerWidget.ts:103-108`).
- **Port:** closes on blur, cursor change, Escape or Close only (`color_picker.rs:890-900`, `:954-962`); `swatch_at` is used only on mouse down (`editor.rs:5600`).
- **Repro:** open the standalone picker, move over another swatch with decorators on hover: both pickers show.
- **Fix:** hide it in mouse move when `swatch_at` hits.

### Code actions, lightbulb and code lenses (#419, #416)

#### C.32 The code action menu stays focused after its line scrolls out of view (P2)
Related to audit 3 §C.23, which fixed opening off screen but not scrolling away after.
- **Monaco:** `toCoords` fixes page coordinates once (`contrib/codeAction/browser/codeActionController.ts:397-422`); the menu stays put and hides on layout change (`:139`).
- **Port:** a text-anchored content widget (`MenuAnchor::Text`, `code_action.rs:1171`, `:1369-1470`), unplaced when its line isn't laid out (`element.rs:5838-5844`), and not hidden on wheel (`editor.rs:6562-6578`) or resize.
- **Repro:** Cmd+. on a diagnostic, wheel the line out of view, Enter: an invisible menu applies an action. Monaco's menu stays visible.
- **Fix:** hide on wheel scroll and resize, or anchor at window coordinates like the lightbulb menu.

#### C.33 The lightbulb and Cmd+. menu use the selection's head line, not its start (P3)
- **Monaco:** the state position is `selection.getStartPosition()` (`contrib/codeAction/browser/codeActionModel.ts:209`, `:373`); bulb and menu go there (`codeActionController.ts:231-237`, `:277`; `lightBulbWidget.ts:474-484`).
- **Port:** `self.cursors.primary().position` (the head) in `schedule_auto_code_actions` (`code_action.rs:1019`) and `trigger_code_actions` (`:1118`).
- **Repro:** select lines 2–6 downward, Cmd+.: Monaco at line 2, the port at line 6.
- **Fix:** use the selection start.

#### C.34 The gutter lightbulb is drawn over code when the glyph margin is off (P3)
- **Monaco:** the gutter bulb is a glyph margin decoration (`lightBulbWidget.ts:362-368`, `:474-484`); with `glyphMargin: false` nothing draws (`browser/viewParts/glyphMargin/glyphMargin.ts:338-341`, `:391-394`).
- **Port:** `LightbulbPlacement::Gutter(row)` with zero glyph width falls through to column 0 (`element.rs:4883-4906`).
- **Repro:** `glyph_margin: false`, unindented `let a`/`let b`/`let c`, a fix on line 2: the bulb is painted over `l`.
- **Fix:** no `Gutter` bulb when `glyph_width == 0`.

#### C.35 Code lenses blank and fade in again after every edit (P3)
- **Monaco:** zones of lines that still have lenses are reused and keep their titles until resolve, with no fade since `_isEmpty` is false (`contrib/codelens/browser/codelensController.ts:340-345`; `codelensWidget.ts:126-131`, `:275-285`).
- **Port:** `render_code_lenses` clears every zone and builds new ones with only resolved lenses (`code_lens.rs:434`, `:440-450`), then `set_lenses(shown, true)` fades them in (`:538`, `:148-157`).
- **Repro:** a provider resolving lazily; type a character: every visible lens blanks and fades back.
- **Fix:** reconcile by row, keep old lenses until resolved, don't animate.

#### C.36 The lightbulb isn't re-requested when providers or the option change (P3)
- **Monaco:** `CodeActionModel._update` runs on model and language changes, `registry.onDidChange` and `lightbulb` option changes (`codeActionModel.ts:197-205`, `:393`).
- **Port:** only cursor and marker changes (`editor.rs:3037`, `:3248`); not `update_options` (`editor.rs:1474+`) or provider registration (`providers.rs:2966-2984`).
- **Repro:** register a code action provider after opening, or switch `lightbulb` Off→On: no bulb until the caret moves.
- **Fix:** schedule from `update_options` and a provider epoch observer, as inlay hints do.

### Peek and marker zone (#406, #418)

#### C.37 F4 / Shift+F4 in the peek only change the preview; the host cursor and zone stay (P2)
- **Monaco:** `goToNextOrPreviousReference` → `_gotoReference(target, false)` hides the widget, sets the host selection and re-shows the peek under the target; other models go to the openers (`contrib/gotoSymbol/browser/peek/referencesController.ts:201-224`, `:248-291`).
- **Port:** `select_relative` only moves the list selection and preview (`peek.rs:651-662`), from `peek_select_relative` (`peek.rs:1710-1716`) and the list's `GoToNextReference` (`:1289-1294`).
- **Repro:** 3 references in one file, Shift+F12 on the first, F4 twice: Monaco's cursor and zone move to ref 2 then 3; the port's stay.
- **Fix:** route F4 through `peek_open_location(target, close: false, …)`; close and use openers for other models.

#### C.38 Enter or double-click on a result after Go to References moves the peek instead of closing it (P2)
- **Monaco:** `peekMode` (`openInPeek`) is false for `goToReferences`, for the "Go to…" commands opening a peek via `gotoLocation.multiple*: peek`, and for `gotoAndPeek`; a `goto` then runs `openReference(…)`, closing the widget (`referencesController.ts:135-141`, `:293-304`, `:389-405`; `goToCommands.ts:180-181`, `:666-669`). Only Peek commands move the widget.
- **Port:** no peek mode; `peek_open_location` re-opens at same-model results whenever `close` is false (`peek.rs:1705-1716` area), and `open_peek` doesn't record its origin (`peek.rs:1538-1593`, `goto.rs:413-433`).
- **Repro:** Shift+F12, select a reference, Enter: Monaco closes and moves the cursor; the port re-opens the peek there.
- **Fix:** keep `open_in_peek` in `PeekState`; when false, close and navigate.

#### C.39 A peek view opened on the last line isn't revealed (P2)
- **Monaco:** `ZoneWidget._showImpl` uses `revealLineNearTop` when the zone's line is the last (`contrib/zoneWidget/browser/zoneWidget.ts:470-482`).
- **Port:** the marker zone handles it (`markers.rs:626-633`), but `open_peek` only does `reveal_range(row..row+1, Simple)` (`peek.rs:1588-1592`).
- **Repro:** a reference on the last line at the bottom of the viewport, Alt+F12: Monaco scrolls it near the top; the port adds the zone below the viewport.
- **Fix:** reuse the marker zone's last-line branch.

#### C.40 `editor.action.findReferences` goes through navigation instead of always opening the peek (P3)
- **Monaco:** calls `toggleWidget` directly (`contrib/gotoSymbol/browser/goToCommands.ts:834-856`): a peek titled "Loading..." (`referencesController.ts:102`) and "No results" inside when empty (`referencesWidget.ts:497-502`).
- **Port:** `FindReferences` runs `peek(References)` (`editor_commands.rs:231-240`) → `navigate` (`goto.rs:340-380`): a "No references found" message and the alternative command for one result; `open_peek` returns on none (`peek.rs:1546-1548`).
- **Repro:** run it on a symbol without references: Monaco opens an empty peek; the port shows a message.
- **Fix:** a dedicated path that opens the peek with an empty state.

#### C.41 Alt+F12 again at the same position re-opens the peek instead of closing it (P3)
- **Monaco:** `toggleWidget` closes and returns when the new range contains the widget's position (`referencesController.ts:77-87`).
- **Port:** `open_peek` always closes and re-opens (`peek.rs:1538-1548`).
- **Repro:** Alt+F12, click back into the host without moving, Alt+F12: Monaco closes; the port re-opens with reset scroll and focus.
- **Fix:** toggle off when the new range contains the open peek's anchor.

#### C.42 F8 / Shift+F8 ("in Files") behave like Alt+F8 and ignore other models' markers (P2)
- **Monaco:** `NextMarkerInFilesAction` navigates a `MarkerList` over every resource (sorted by resource, severity, position); a marker in another resource closes the zone and opens it through `openCodeEditor`, i.e. editor openers in standalone (`contrib/gotoError/browser/gotoError.ts:144-172`, `:240-282`; `markerNavigationService.ts:51-72`, `:122-174`).
- **Port:** both run `go_to_marker` (`editor.rs:7641-7646`) over `model.navigable_markers()` (`markers.rs:505-526`). `book/src/keymap.md:17` says they "act the same within one model", which no longer holds now that markers are kept by uri across models (`MONACO_PARITY.md:140`).
- **Repro:** `a.ts` with 2 errors, markers on `file:///b.ts` with 3; F8 in `a.ts`: Monaco "1 of 5 problems", then goes to `b.ts`; the port "1 of 2" and wraps.
- **Fix:** build the list from `model_markers` across uris for the InFiles actions; open other resources with `open_location_with_openers`.

#### C.43 Marker zone header layout and the missing header/body separator (P3)
- **Monaco:** a `PeekViewWidget` (`contrib/gotoError/browser/gotoErrorWidget.ts:230`): a 1px `border-top` on the body in the frame color (`contrib/peekView/browser/media/peekViewWidget.css`, `peekView.ts:155-170`), title `margin-left: 20px`, icon 4px before the name, and "1 of 2 problems" as `.dirname` at 0.9em with a `.5em` margin (`gotoErrorWidget.ts:360`).
- **Port:** no separator (`markers.rs:1028-1032`); header `.px(8.)`, `.gap(6.)`, 13px detail (`markers.rs:913-939`). The port's own peek draws the separator (`peek.rs:1452-1456`).
- **Repro:** Alt+F8 on an error.
- **Fix:** `border_t_1` on the body, `ml(20)` title, `mr(4)` icon, an 11.7px detail with `ml(6.5)`.

### Inline edits and the paste chip (#430)

#### C.44 The docked inline-edit gutter indicator uses arrows instead of `keyboard-tab-above/below` (P3)
- **Monaco:** docked with Tab not accepting → `cursorLine <= editStartLine ? keyboardTabAbove : keyboardTabBelow`, never rotated (`contrib/inlineCompletions/browser/view/inlineEdits/components/gutterIndicatorView.ts:356-368`, `:390-395`).
- **Port:** `ArrowDown`/`ArrowUp` while docked (`inline_completions/inline_edits_view.rs:669-689`); the codicons exist (`codicons.rs:1005-1006`). `differences.md:51` covers only the undocked icon's rotation.
- **Repro:** an inline edit below the cursor with the pill docked: Monaco's tab-above glyph against the port's ↓.
- **Fix:** map to `KeyboardTabAbove`/`KeyboardTabBelow` when docked.

#### C.45 The inline toolbar's warning row draws an icon Monaco doesn't (P3)
- **Monaco:** `div.warningMessage` holds only the message (`contrib/inlineCompletions/browser/hintsWidget/inlineCompletionsHintsWidget.ts:182-203`); `warning.icon` is for ghost text only (`view/ghostText/ghostTextView.ts:51`).
- **Port:** a codicon before the message (`inline_completions/toolbar.rs:345-362`).
- **Repro:** `show_toolbar: Always`, a completion with a plain `warning`.
- **Fix:** drop the icon.

#### C.46 The paste/drop options button is anchored at the primary cursor, not the edit's end (P3)
- **Monaco:** `PostEditWidget.getPosition` is the end of the tracked range over `ranges[0]`, `BELOW` (`contrib/dropOrPasteInto/browser/postEditWidget.ts:118-123`, `:231-255`).
- **Port:** `self.cursors.primary().position` after applying (`drop_paste.rs:1011-1024`); for snippets that's the first tabstop.
- **Repro:** two paste providers, one returning `[${1:text}](url)`: Monaco's button sits under the end of `(url)`, the port's under `text`.
- **Fix:** track `ranges[0]` with a growing decoration and anchor at its end.

#### C.47 Hard-coded keybinding labels in the inline edit views (P3)
Related to audit 3 §C.50.
- **Monaco:** the word replacement view's alternative label and the jump-to label look up `inlineSuggestCommitAlternativeAction` and `jumpToNextInlineEdit` (`contrib/inlineCompletions/browser/view/inlineEdits/inlineEditsViews/inlineEditsWordReplacementView.ts:153`, `:295-297`; `jumpToView.ts:85-89`, `:166-170`, `:200-216`).
- **Port:** a literal `"⇧⇥"` (`inline_completions/inline_edits_view.rs:456`, on Linux and Windows too) and a "Tab" keycap (`:530-537`).
- **Repro:** rebind those commands, or run on Linux.
- **Fix:** `inline_command_binding` plus `keycaps_styled`, as the §C.50 fix does.

## D. Navigation, scrolling and folding

Scope: reveal (`browser/viewParts/viewLines/viewLines.ts`), `revealAll`, the scroll commands, `editorScroll`, `revealLine` and mouse `MoveTo` (`browser/coreCommands.ts`), `base/common/scrollable.ts` and the shipped wheel handling (monaco-editor 0.55.1 `scrollableElement.js`, upstream, not vendored), `contrib/gotoSymbol`, the standalone code editor service, bracket matching, word highlights, anchor select, cursor undo, folding and hidden ranges, sticky scroll (spot checks), section headers, and view state save and restore.

Checked and matching (the audit 3 §D fixes hold): `reveal.rs` including `minimalReveal` and `cursorSurroundingLinesStyle`, the reveal API family and `ScrollType` (a reveal request of a line or less jumps), `editorScroll` with `revealCursor`, `pageSize`, `scrollEditorBottom`, composed long smooth scrolls with the 10ms head start, wheel smooth scrolls (0.55.1's `_onMouseWheel` calls `setScrollPositionSmooth(desired)` without `reuseAnimation`, which the port's restart-from-current `combine` matches), fold commands and `FoldingArguments`, `revealCursor` unfolding, `adjustSelections`, chevron mouse-up handling, the folding memento, sticky line numbers, chevrons and `showEndForLine`, section header detection, cursor undo, the selection anchor, F7 word-highlight navigation, and the Go to/Select to Bracket fallbacks.

### Go to Definition and References (#401, #406)

#### D.1 Go to Definition runs the alternative command when the cursor is anywhere inside the definition's full range (P1)
- **Monaco:** `references.referenceAt(model.uri, position)` decides "already at the only result", which runs the alternative (Go to References) (`contrib/gotoSymbol/browser/goToCommands.ts:126-133`, `:146`). `referenceAt` tests `OneReference.range` (`referencesModel.ts:275-283`), which is `targetSelectionRange ?? range` (`referencesModel.ts:39-41`): the symbol's name for a `LocationLink`. The same range drives the F12 "still on a result" check (`symbolNavigation.ts:95-105`) and `nearestReference` (`referencesModel.ts:247-253`).
- **Port:** `navigate_from` computes `at_cursor` with `location.range`, the full target range (`goto.rs:349-353`); `check_symbol_navigation` (`goto.rs:540-544`) and the peek's `nearest` (`peek.rs:179-191`) do too. `Location::selection_range()` exists (`providers.rs:701-705`), and the LSP bridge fills `target_selection_range` (`crates/gpui-monaco-lsp/src/convert.rs:352`).
- **Repro:** a provider returning a `LocationLink` (rust-analyzer or tsserver through `gpui-monaco-lsp`), `fn fact(n: u64) -> u64 { if n == 0 { 1 } else { n * fact(n - 1) } }` (or TS `class Foo { static make() { return new Foo(); } }`), cursor on the inner `fact`/`Foo`, F12. Monaco jumps to the name on the first line; the port finds the single result's range contains the cursor and opens a references peek. F12 stepping through results also stays active while the cursor wanders inside a result's body.
- **Fix:** use `location.selection_range()` for `at_cursor`, `check_symbol_navigation` and `nearest`.

#### D.2 The navigation alternative inherits the original command's peek mode, and can't name the peek variant (P3)
- **Monaco:** the alternative is another registered action (`gotoLocation.alternativeDefinitionCommand` and friends are command ids, default `editor.action.goToReferences`, `common/config/editorOptions.ts:2249-2253`) and runs with its own `SymbolNavigationActionConfig` (`goToCommands.ts:126-149`): `editor.action.goToReferences` has `openInPeek: false, muteMessage: false` (`:664-669`), `editor.action.referenceSearch.trigger` `openInPeek: true` (`:705-710`).
- **Port:** the alternative is a `LocationKind` (`options.rs:1410-1437`, `:1471-1479`), and `navigate_from` re-runs it with the original `open_in_peek` and `mute_message` (`goto.rs:367-378`).
- **Repro:** Peek Definition (Alt+F12) on an unused function's own name: Monaco runs Go to References with `openInPeek: false` and, with one result, opens no peek; the port opens a one-item references peek. An option naming the peek references command as the alternative can't be expressed.
- **Fix:** model the alternative as (kind, `open_in_peek`) and run it with that command's config.

#### D.3 Jumps within the same model reveal "near top, smooth" instead of standalone Monaco's "center, immediate" (P3)
- **Monaco (standalone):** Go to Definition/References results, F12 stepping and peek "goto" open through `ICodeEditorService.openCodeEditor`; `StandaloneCodeEditorService.doOpenEditor` ignores `selectionRevealType` and does `setSelection(selection); revealRangeInCenter(selection, ScrollType.Immediate)` (`standalone/browser/standaloneCodeEditorService.ts:78-90`), so the target is centered even when visible, without animation.
- **Port:** `open_location` uses `reveal_range(start..start, NearTopIfOutsideViewport)` with the default smooth scroll type (`goto.rs:448-464`); `peek_open_location` uses `RevealType::Center` but smooth (`peek.rs:1749-1751`). `book/src/providers.md:140` documents "revealed near the top", but `differences.md` doesn't list it.
- **Repro:** F12 on a call whose definition is 3 lines below and visible: Monaco re-centers, the port doesn't scroll. With `smooth_scrolling`, F12 to a far definition: Monaco jumps, the port animates.
- **Fix:** reveal with `RevealType::Center` and `ScrollType::Immediate` in both, or document it as a deliberate VS Code-like difference.

#### D.4 Peek "open" (title click, double-click in the preview) leaves a caret instead of selecting the reference (P3)
- **Monaco:** kind `'open'` (`contrib/gotoSymbol/browser/peek/referencesWidget.ts:330-332`, `:521-523`) → `ReferencesController.openReference` passes the whole `OneReference.range` as `selection` (`peek/referencesController.ts:293-303`), which the standalone service applies with `setSelection(range)` (`standaloneCodeEditorService.ts:80-82`). Enter (`_gotoReference`) collapses to the start (`:248-252`).
- **Port:** both paths set `Selection::collapsed(location.range.start)` (`peek.rs:1739-1751`).
- **Repro:** Shift+F12, double-click a line in the preview (with `stable_peek` off) or click the title: Monaco closes with the reference selected; the port leaves a caret at its start.
- **Fix:** for `close` in the same model, `set_selections(&[Selection::new(range.start, range.end)])`.

### Scrolling and reveal (#411)

#### D.5 Mouse clicks reveal the cursor horizontally with 30px/right padding instead of minimally (P2)
- **Monaco:** a click is `_moveTo` → `revealAllCursors(source, true, minimalReveal = true)`, only if the cursor state changed (`browser/coreCommands.ts:394-403`; `_wordSelect` `:1653`). With `minimalReveal`, `_computeScrollLeftToReveal` adds neither `HORIZONTAL_EXTRA_PX` (30px) nor `revealHorizontalRightPadding` (`browser/viewParts/viewLines/viewLines.ts:819-822`), so clicking a visible column never scrolls sideways.
- **Port:** `dispatch_mouse` sets `autoscroll_minimal = true` (`editor.rs:5872-5875`), and the vertical path honors it (`element.rs:3603`), but the single-cursor horizontal autoscroll always pads 30px left and `reveal_horizontal_right_padding` right (`element.rs:3916-3931`). Only the multi-cursor `pending_reveal_x` path passes `minimal` (`element.rs:6738`). A click that doesn't change the selection still requests the reveal (`editor.rs:3214-3223`).
- **Repro:** a 300-column line scrolled to `scroll_left` 600px; click 10px from the text area's left edge: the view jumps 20px left. Click within 15px of the right edge (or double-click a word there): it scrolls right. Monaco scrolls in neither case.
- **Fix:** use `(0, 0)` padding in the single-cursor branch when `autoscroll_minimal` (or go through `scroll_left_to_reveal` with `minimal`).

#### D.6 Cursor autoscrolls of a line or less animate with `smooth_scrolling` (P2)
- **Monaco:** every cursor reveal goes through `onRevealRangeRequest`, which jumps when the scroll moves a line or less: `scrollType = scrollTopDelta <= lineHeight ? Immediate : e.scrollType` (`browser/viewParts/viewLines/viewLines.ts:296-297`).
- **Port:** the rule applies only to `pending_reveal` (the reveal APIs, `element.rs:3554-3560`). The cursor autoscroll path (`element.rs:3584-3640`) sets `scroll_top` while `cursors_changed` has set `smooth_scroll_requested = true` (`editor.rs:3230-3236`), so `scroll_top_position` starts a 135ms ease-out for any distance (`view/animation.rs:331-342`, `:316-322`). Only the reveal API case is tested (`crates/gpui-monaco/tests/editor_events.rs:621-633`).
- **Repro:** `smooth_scrolling: true`, caret on the last visible line, Down (or hold it): Monaco scrolls one line instantly per press; the port glides each step, so held arrows lag behind the caret.
- **Fix:** in the autoscroll branches compare the new top with `scroll_before.y` and `apply_scroll_type(Immediate)` when the delta is `<= line_height`.

#### D.7 Inertial scrolling also runs for physical mouse wheels (P3)
- **Monaco (upstream, not vendored; monaco-editor 0.55.1 `scrollableElement.js` `_onMouseWheel`):** inertia starts only `if (inertialScroll && (deltaX || deltaY) && !classifier.isPhysicalMouseWheel())`; physical wheels get the smooth scroll instead.
- **Port:** `scroll_by_wheel` feeds every delta, `ScrollDelta::Lines` included, to `inertial.on_wheel` (`editor.rs:6656-6660`), while treating `Lines` as the physical wheel for smoothing (`:6638-6655`).
- **Repro:** `inertial_scroll: true`, `smooth_scrolling: true`, one wheel notch: Monaco smooth-scrolls 3 lines and stops; the port coasts for ~40 more frames.
- **Fix:** skip `inertial.on_wheel` for `ScrollDelta::Lines`.

#### D.8 `revealLine` parses a string `lineNumber` strictly (P3)
Same root as audit 3 §C.42 (Go to Line).
- **Monaco:** `parseInt(lineNumberArg) + 1` (`browser/coreCommands.ts:1843-1844`), so `"12px"` or `" 12.9"` reveal line 13 (1-based).
- **Port:** `s.trim().parse::<i64>().unwrap_or(0)` (`editor_commands.rs:811-815`): such strings reveal the first line.
- **Repro:** `execute_command("revealLine", {"lineNumber": "12px"})`.
- **Fix:** parse the leading integer like `parseInt`.

### Word highlights and bracket commands (#410, #422)

#### D.9 Textual word highlights aren't capped at 999 matches (P3)
- **Monaco:** `TextualDocumentHighlightProvider` calls `model.findMatches(word, true, false, true, USUAL_WORD_SEPARATORS, false)` with the default `limitResultCount = LIMIT_FIND_COUNT = 999` (`contrib/wordHighlighter/browser/textualHighlightProvider.ts:37`, `:67`; `common/model/textModel.ts:125`, `:1178`).
- **Port:** `textual_highlights` uses `find::MATCHES_LIMIT`, 19,999 (`occurrences.rs:33-49`, `find.rs:26`).
- **Repro:** 5,000 occurrences of `x`: Monaco highlights (and F7 cycles through) the first 999; the port all 5,000.
- **Fix:** pass 999 for the word-highlight searches.

#### D.10 Remove Brackets makes one undo step for all cursors (P3)
- **Monaco:** `removeBrackets` wraps each selection's `executeEdits` in `pushUndoStop()` before and after (`contrib/bracketMatching/browser/bracketMatching.ts:273-298`), so each cursor's removal is its own undo step.
- **Port:** one `EditOperation` for every pair (`bracket_matching.rs:76-92`).
- **Repro:** two cursors in different `( … )` pairs, Cmd/Ctrl+Alt+Backspace, Cmd+Z: Monaco restores only the second pair; the port both.
- **Fix:** push a stack element and apply each cursor's pair separately, keeping the dedup.

## E. Diff editor and multi diff editor

Scope: diff computation (advanced, legacy, moves, normalization), side-by-side and inline rendering, view zones and alignment, hidden unchanged regions, revert, the gutter and overview ruler, the accessible diff viewer, scroll sync, options and validation, events and API, and the multi diff editor (list, headers, scrolling, view state, documents). Monaco paths starting `browser/widget/...` are under `vs/editor/`; `features/` and `components/` are under `browser/widget/diffEditor/`.

`cargo test -p gpui-monaco --test diff_editor --test multi_diff_editor --test diffing` passes (78, 30 and 2 tests; the fixture suites cover all 58 fixture directories), and the diff algorithms still match Monaco line for line. Audit 3 §E.1, §E.2, §E.4, §E.7, §E.8, §E.10, §E.12–§E.14, §E.17 and §E.19–§E.24 were re-checked; §E.11, §E.16, §E.20/§E.24 and §E.22 are incomplete (§E.3, §E.10, §E.21, §E.22).

### Accessible diff viewer (#403, #431)

#### E.1 The accessible diff viewer gives screen readers nothing to read (P1)
- **Monaco:** the current row gets `tabIndex = 0` and DOM focus with `row.focus()` (`browser/widget/diffEditor/components/accessibleDiffViewer.ts:520-533`); each row is a `listitem` with its own `aria-label` (`:572-573`, `:637-653`). F7 and every Up/Down make the screen reader read the new row, e.g. "+ foo modified line 5".
- **Port:** focus goes to the viewer's root, which has an id and `track_focus` but no `.role()` (`diff_editor/accessible.rs:528-532`; focused from `diff_editor.rs:5629`, `:5639`). GPUI makes an accessibility node only for an element with a role and logs "focus without node … it has an id but no role" (gpui-pre 0.3.8 `src/window/a11y.rs:248-254`), so focus falls back to the window. The rows have `Role::ListItem` and labels (`accessible.rs:430`, `:497`), but the selected row is only a border (`:517-521`): no `aria_active_descendant` or `aria_selected`, and nothing is announced.
- **Repro:** VoiceOver on, focus the modified editor, F7, Down three times. Monaco reads the header ("Difference 1 of 2…") and each line; the port reads nothing.
- **Fix:** move `track_focus` onto the `Role::List` element (`accessible.rs:552`) or give the root a role, and put `.aria_active_descendant().aria_selected(true)` on the current row.

#### E.2 Accessible diff viewer state survives re-applies and model changes (P3)
- **Monaco:** groups are rebuilt only when the diff changes; the element index is kept and only the group is re-picked from the cursor (`components/accessibleDiffViewer.ts:137-161`). `setModel(null)` closes the viewer (`browser/widget/diffEditor/diffEditorWidget.ts:581-584`), and new models start with an empty list.
- **Port:** `set_diffs` resets `current_element = 0` and re-picks the group on every `apply_diff` (`diff_editor/accessible.rs:184-199`; from `diff_editor.rs:3627-3630`), including `update_options`, theme changes, layout switches and the end of a moved-block compare. `set_model`/`clear_diff` never touch the viewer (`diff_editor.rs:2597-2617`, `:3547-3564`).
- **Repro:** F7, Down to group 1's last context line, change the theme: Monaco stays; the port shows group 2's header. F7, then `set_model(None)`: Monaco closes the viewer; the port keeps it with blank rows.
- **Fix:** call `set_diffs` only when the diff `Arc` changes, keeping `current_element` (clamped); close or clear the viewer in `set_model`/`clear_diff`.

#### E.3 `diffEditorAutomaticRenderSideBySide` ignores screen reader mode (P3)
Incomplete fix of audit 3 §E.11.
- **Monaco:** `renderSideBySideInAutomaticMode = width > breakpoint || screenReaderMode`, after the compact-mode check (`browser/widget/diffEditor/diffEditorOptions.ts:41-47`).
- **Port:** `!(compact && simple) && width > breakpoint` (`diff_editor.rs:3157-3161`).
- **Repro:** screen reader on, an 800px diff editor: Monaco's key is true, the port's false.
- **Fix:** add `|| self.screen_reader_mode`.

#### E.4 The viewer header doesn't copy Monaco's swapped ranges (P3)
Decide and document (#431).
- **Monaco:** builds `new LineRangeMapping(modifiedRange, originalRange)` (`components/accessibleDiffViewer.ts:279-282`), so its header prints the sides reversed: deleting original lines 10–12 shows `@@ -10,0 +10,3 @@`.
- **Port:** prints the correct `@@ -10,3 +10,0 @@` (`diff_editor/accessible.rs:404-423`).
- **Fix:** keep the port's header and list it in `book/src/differences.md`.

### Hidden unchanged regions (#404)

#### E.5 Restoring unchanged regions after an edit can panic in debug builds (P2)
- **Monaco:** `updateUnchangedRegions` reads the old regions back from their tracking decorations and cleans them before `inverse` (`browser/widget/diffEditor/diffEditorViewModel.ts:165-193`): `filterWithPrevious` drops a region overlapping the one before (`:173-187`; helper `browser/widget/diffEditor/utils.ts:433-440`), and `LineRangeMapping.clip` drops a mapping whose clipped range is empty (`common/diff/rangeMapping.ts:46-56`).
- **Port:** both are missing. `tracked_unchanged_regions` keeps every tracked region (`diff_editor.rs:4631-4659`); `transfer_state`'s `clip` turns an empty or missing intersection into `LineRange::new(1, 1)` (`diff_editor/unchanged.rs:308-312`). `inverse` (`unchanged.rs:30-45`) can then build `LineRange::new(prev_end, next_start)` with start > end, which trips the `debug_assert` in `diff/range.rs:199-203`; release builds get an inverted range.
- **Repro:** 100-line models with changes at lines 30 and 70, `hide_unchanged_regions` on. Cursor on line 32, Shift+Ctrl+End, type a character. The two later regions' decorations collapse onto line 32 and clip to the same `[32,33)`. Monaco drops the duplicate; the port's next recompute reaches `inverse` with `LineRange::new(33, 32)` and panics in debug builds (examples, `cargo test`).
- **Fix:** port `filterWithPrevious` into `tracked_unchanged_regions`; in `transfer_state`, `filter_map` and drop a mapping when either intersection is missing or empty.

#### E.6 New hidden regions start collapsed when the previous diff had none (P2)
- **Monaco:** `_unchangedRegions` is `undefined` only before the first diff. After that an empty list means "everything is visible": `visibleRegions = inverse([])` covers the file, so new regions come out revealed (`diffEditorViewModel.ts:166-201`). A diff with no changes keeps its whole-file region in the model; only the view hides it (`features/hideUnchangedRegionsFeature.ts:95-100`).
- **Port:** `transfer_state` returns the new regions collapsed whenever `previous.is_empty()` (`diff_editor/unchanged.rs:305-307`), and `update_unchanged_regions` stores `Vec::new()` for the whole-file case (`diff_editor.rs:4613-4619`), so "first diff", "no hideable regions" and "nothing changed" all act like "first diff".
- **Repro:** 30-line models with changes at 5, 10, 15, 20 and 25 (every gap too short to hide); revert 10, 15 and 20. Monaco shows the new region [9,22) revealed with only the fold glyph; the port collapses the 13 lines you were reading into a bar. The same happens when a host fills empty models after creating the editor.
- **Fix:** keep the previous state as `Option<Vec<UnchangedRegion>>` (`Some(vec![])` = whole file visible); keep the whole-file region in the state and skip it only when applying bars.

#### E.7 Inline view: the original column's hidden-lines bars show the unfold button and label (P3)
- **Monaco:** the original editor's overlay gets `_hide = !sideBySide` (`features/hideUnchangedRegionsFeature.ts:135`, `:158`), so it has no buttons, label or breadcrumbs (`:346-357`, `:499`), and the compact widget empties itself (`:297`).
- **Port:** `apply_unchanged` builds the same `UnchangedBar` on both sides (`diff_editor.rs:4737-4760`) with no hide flag; the unfold button is centered in the original editor's text area, inside the visible line-number column (`:1189-1208`).
- **Repro:** `render_side_by_side: false` with `hide_unchanged_regions`: a second unfold icon in the line-number column; with `compact_mode`, a clipped rule and label.
- **Fix:** a `hide` flag for the original side when inline that renders only the background and edges.

#### E.8 `compact_mode` drops the "Fold Unchanged Region" glyph (P3)
- **Monaco:** the glyph is skipped only for the `compact` variant (multi diff cards), not the `compactMode` option (`features/hideUnchangedRegionsFeature.ts:211`, `:229`).
- **Port:** `apply_fold_glyphs` returns early with `options.compact_mode` (`diff_editor.rs:3064-3066`).
- **Repro:** `compact_mode` with hidden regions, Show All: Monaco offers a fold glyph per region; the port can't re-fold one.
- **Fix:** remove the `compact_mode` early return.

### High contrast and inline deleted lines (#421)

#### E.9 The diff overview ruler is empty in high-contrast themes (P2)
- **Monaco:** the insert color is `diffEditorOverview.insertedForeground`, else `diffEditor.insertedTextBackground`, else `defaultInsertColor` (rgba(155,185,85,.2)), alpha doubled; removals use `defaultRemoveColor` (rgba(255,0,0,.2)) the same way (`features/overviewRulerFeature.ts:45-51`).
- **Port:** doubles the alpha of `diff_inserted_text_background`/`diff_removed_text_background` with no constant fallback (`diff_editor.rs:4418-4433`). In hc-black and hc-light those and the overview colors are null (`theme/color_registry.rs:347-348`, `:353-354`), so the marks are transparent.
- **Repro:** `Theme::hc_black()`, any diff: Monaco shows green and red marks; the port only the slider.
- **Fix:** fall back to Monaco's constants before doubling the alpha.

#### E.10 The inline deleted-lines zone has no high-contrast borders (P3)
Incomplete fix of audit 3 §E.16.
- **Monaco:** the zone node is `line-delete` and changed characters are `char-delete` spans, both with a 1px `diffEditor.removedTextBorder`, dashed in HC (`components/diffEditorViewZones/diffEditorViewZones.ts:210`; `browser/widget/diffEditor/style.css:391-399`).
- **Port:** `DeletedCode` paints backgrounds only (`diff_editor.rs:1705-1716`, `:1807`).
- **Repro:** hc-black, inline view: Monaco outlines the deleted block and changed characters with dashes; the port doesn't.
- **Fix:** draw dashed `diff_removed_text_border` boxes around the zone's lines and changed runs when the theme sets it.

#### E.11 Inline deleted lines without inner changes lack the full-width `char-delete` tint (P3)
- **Monaco:** with no character decorations, each rendered line gets a full-width `char-delete` span, layering `removedTextBackground` on the line background (`components/diffEditorViewZones/renderLines.ts:26`, `:277-280`).
- **Port:** only `inner_changes` are highlighted (`diff_editor.rs:1705-1716`), and that list is empty for legacy pure deletions (§E.19).
- **Repro:** `diff_algorithm: Legacy`, inline, delete 3 lines: Monaco's zone is darker red; the port's has only the line background.
- **Fix:** with no inner changes, add a full-width `diff_removed_text_background` layer.

#### E.12 Inline deleted lines ignore `render_whitespace`, `render_control_characters` and `font_ligatures` (P3)
- **Monaco:** `RenderOptions.fromEditor` passes the modified editor's options to `renderViewLine` (`components/diffEditorViewZones/renderLines.ts:103-126`, `:292-316`).
- **Port:** `DeletedCode::render` only expands tabs (`diff_editor.rs:1663`).
- **Repro:** `render_whitespace: All`, inline: Monaco's deleted lines show whitespace marks; the port's don't.
- **Fix:** reuse the editor's whitespace and control-character run builders for the deleted text.

#### E.13 The original side has no high-contrast border or shadow, and the gutter has no scroll shadow (P3)
- **Monaco:** side by side, `.editor.original` gets `border-right: 1px diffEditor.border` and a mirrored `scrollbar.shadow` (`browser/widget/diffEditor/style.css:428-431`); the gutter adds `.scroll-decoration` when `scrollTop > 0` (`browser/widget/diffEditor/utils/editorGutter.ts:42-59`).
- **Port:** only the modified container gets a border and shadow (`diff_editor.rs:~6598-6620`; the original's container at `:6545-6549` has neither), and `render_gutter` draws no scroll shadow (`:5574`).
- **Repro:** hc-black with the gutter menu: contrast lines on both sides of the gutter in Monaco, one in the port. Scrolling down leaves a 35px gap in the top shadow over the gutter.
- **Fix:** add the border and shadow to the original container; draw the scroll-decoration strip in the gutter when `scroll_top > 0`.

### Gutter, layout and API polish (#424)

#### E.14 Gutter toolbar items leave out the zones next to a change (P3)
- **Monaco:** the item spans `getTopForLineNumber(start, true)` to `getBottomForLineNumber(endEx-1, true)`, including the zones just above and below (the inline deleted-lines zone, the side-by-side padding) (`browser/widget/diffEditor/utils/editorGutter.ts:124-133`); `DiffToolBar.layout` centers on that span and measures `isSmall` against it (`features/gutterFeature.ts:287-318`).
- **Port:** `render_gutter` uses `rows_y_span`, the text lines only (`diff_editor.rs:5478`, `:5490`, `:6002-6022`).
- **Repro:** inline, 6 deleted lines replaced by 1: Monaco's bar spans 7 rows, the port's 1; at line 1 the item counts as small (19px < 30px) and shows 1 button instead of 3.
- **Fix:** add the zone heights above the first line and below the last.

#### E.15 Moved-block connector lines sit about 9px too low (P3)
- **Monaco:** `computeLineStart` averages `getTopForLineNumber(start, true)` and `getTopForLineNumber(endEx, true)`, including the 18px "Code moved…" caption zone (`features/movedBlocksLinesFeature.ts:73-77`).
- **Port:** the middle of `rows_y_span(lines)`, which starts below the caption (`diff_editor.rs:4277-4280`).
- **Repro:** `show_moves` with a 1-line moved block: Monaco's arrow is near the line's top edge, the port's at its middle.
- **Fix:** start the span at the top of the zones above the first line.

#### E.16 Revert arrows and fold glyphs use the wrong glyph lane and z-index (P3)
- **Monaco:** `RevertButton.getPosition` uses lane `Right` with `zIndex: 10001` (`features/revertButtonsFeature.ts:169`, `:176`); the fold-unchanged glyph uses `zIndex: 10001` (`features/hideUnchangedRegionsFeature.ts:200`).
- **Port:** both use `GlyphMarginWidget::new`, the center lane at z 0 (`widgets.rs:636-642`; `diff_editor.rs:5121`, `:3086`).
- **Repro:** `render_gutter_menu: false` and a host glyph decoration with `z_index > 0` on a changed line: Monaco shows both, the port hides the arrow.
- **Fix:** `lane = Right, z_index = 10001` for arrows, `z_index = 10001` for fold glyphs.

#### E.17 Missing modified-editor overrides: reveal padding and scrollbar arrows (P3)
- **Monaco:** the modified side gets `revealHorizontalRightPadding = 15 + 30` and `scrollbar.verticalHasArrows = false` (`browser/widget/diffEditor/components/diffEditorEditors.ts:174-175`, `:184`).
- **Port:** `apply_editor_options` sets neither (`diff_editor.rs:2667-2726`); the padding stays 15 (`options.rs:2176`), and a host's `vertical_has_arrows` stays on though the overview slider assumes none.
- **Repro:** cursor at the end of a long modified line: Monaco keeps 45px to its right, the port 15px.
- **Fix:** force both on the modified side, saving the host's values as the sticky scroll override does.

#### E.18 Changing `split_view_default_ratio` throws away a dragged sash (P3)
- **Monaco:** `sashLeft` uses `_sashRatio ?? splitViewDefaultRatio`, and only a reset clears `_sashRatio` (`browser/widget/diffEditor/components/diffEditorSash.ts:11-24`).
- **Port:** `update_options` overwrites `self.ratio` whenever the default changes (`diff_editor.rs:2562-2564`).
- **Repro:** drag the sash to 30%, set the default to 0.6: Monaco stays at 30%, the port jumps to 60%.
- **Fix:** store the ratio as `Option<f32>`, `None` following the default.

#### E.19 Legacy diffs: "no inner changes" becomes an empty list instead of undefined (P3)
- **Monaco:** `innerChanges` is `undefined` for legacy changes of 20+ lines and its empty-side paths (`common/diff/legacyLinesDiffComputer.ts:463-496`), so `toLineChanges` gives `charChanges: undefined` (`browser/widget/diffEditor/diffEditorWidget.ts:850-892`), and margin revert arrows need `innerChanges` (`features/revertButtonsFeature.ts:61`). For an empty original, the provider's fast path returns one inner change covering everything before either algorithm runs (`browser/widget/diffEditor/diffProviderFactoryService.ts:80-108`).
- **Port:** `compute_legacy_diff` stores `inner_changes.unwrap_or_default()` (`diff/legacy.rs:164`), so `to_line_changes` gives `Some([])` (`diff_editor.rs:2004`) and revert arrows show for every change (`:5104`); there's no empty-original fast path.
- **Repro:** `diff_algorithm: Legacy`, an empty original and 3 modified lines: Monaco highlights the inserted text as char-insert and `charChanges` is undefined; the port shows line backgrounds only and `char_changes` is `Some([])`.
- **Fix:** add the fast path in `DiffEditor::compute_diff`; track whether inner changes were computed and use it in `to_line_changes` and `apply_revert_arrows`.

#### E.20 Smaller diff editor API gaps: old viewer ids, a pending reveal across `set_model` (P3)
- **Monaco:** registers `editor.action.diffReview.next`/`.prev` as aliases (`browser/widget/diffEditor/diffEditor.contribution.ts:82`, `:85`). `revealFirstDiff` returns at once without a model and waits only on the captured view model (`diffEditorWidget.ts:758-771`).
- **Port:** `DiffEditor::trigger` (`diff_editor.rs:5769-5803`) and `ALIASES` (`editor_actions.rs:610-631`) lack the aliases, so they're unknown commands. `reveal_first_diff` sets `reveal_first_diff_pending` (`diff_editor.rs:5809-5816`), which only the next diff clears.
- **Repro:** `set_model(None)`, `reveal_first_diff()`, then `set_models(a, b)`: the port reveals in the new models.
- **Fix:** add the aliases; clear the pending reveal in `set_model` and at the `max_file_size` cut-off.

### Multi diff editor (#407, #425, #431)

#### E.21 A view state restored before or with loading is lost, and focus doesn't follow it (P2)
Incomplete fix of audit 3 §E.20 and §E.24.
- **Monaco:** `setViewModel(vm, { viewState, preserveFocus })` applies documents and view state together (`browser/widget/multiDiffEditor/multiDiffEditorWidget.ts:123-151`). States for files not loaded yet wait in `_lastDocStates` (`multiDiffEditorWidgetImpl.ts:149-155`, `:515`); a saved active file suppresses the "Initialize first change" autorun (`:314-318`); `_restorePendingActiveDiffItem` focuses it unless `preserveFocus` (`:549-571`).
- **Port:** `restore_view_state` only touches files already listed and keeps nothing for later ones (`multi_diff_editor.rs:1399-1440`). `set_documents` always goes to the first change when no file is active (`:739-742`), and that path ends in `reveal` → `set_scroll_top`, clearing `pending_scroll`. A restore never moves focus.
- **Repro:** `set_documents_loading()`, `restore_view_state(&saved)`, `set_documents(docs)`: Monaco restores collapsed states, selections, the active file and scroll; the port loses all of it (file 0 expanded and focused, cursor on its first change, scroll 0). `set_documents(docs)` then `restore_view_state` with file 3 active in one update: Monaco focuses file 3; the port stays in file 0, and the render focus check can make file 0 active again.
- **Fix:** keep pending doc states and the active key across `set_documents_loading`/`replace_documents`; skip first-change navigation when a pending active key matches; focus the restored file unless `preserve_focus_on_load`.

#### E.22 Per-file options can't carry editor options such as read-only (P2)
Incomplete fix of audit 3 §E.22.
- **Monaco:** `IDocumentDiffItem.options` is an `IDiffEditorOptions` (`browser/widget/multiDiffEditor/model.ts:38`), which extends the editor options (`readOnly`, `accessibilitySupport`, fonts…); the template applies it with `editor.updateOptions` (`diffEditorItemTemplate.ts:372-391`, `:442`). The vendored widget test depends on a per-file `accessibilitySupport: 'off'`.
- **Port:** `MultiDiffDocument::options` is a `DiffEditorOptions` without editor options (`diff_editor.rs:134-221`), and each bind's `configure_template` sets the modified editor's `read_only = deleted` (`multi_diff_editor.rs:1024-1043`), so a host's `read_only` on a pooled editor is undone at the next bind and leaks to whichever file reuses it.
- **Repro:** make file 0 read-only through `diff_editor(0)`, scroll it out and back: editable again. Monaco's `options: { readOnly: true }` sticks.
- **Fix:** a per-document editor-options layer (at least `read_only`), applied in `configure_template` and reset on unbind.

#### E.23 No list-wide layout override (`setRenderSideBySide`, `setViewMode`, `setDiffWordWrap`) (P2)
- **Monaco:** these set `_diffLayoutOptions` (`multiDiffEditorWidget.ts:164-209`), which forms `_optionsOverride` (`multiDiffEditorWidgetImpl.ts:84-86`), applied after each file's own options (`diffEditorItemTemplate.ts:374-376`), and drives the `multiDiffEditorRenderSideBySide` and `diffEditorAutomaticRenderSideBySide` keys (`multiDiffEditorWidgetImpl.ts:221-254`).
- **Port:** `update_options` only reaches files without their own options (`multi_diff_editor.rs:1134-1161`); only `set_hide_original_line_numbers` overrides a file's options; neither key exists.
- **Repro:** give one file its own `options`, then switch every file to inline: Monaco's `setRenderSideBySide(false)` switches all; the port leaves that file side by side.
- **Fix:** a layout override (side by side, inline when space is limited, word wrap) merged last in `item_options`, and the two keys on the list.

#### E.24 A kept file with new models gets its old view state back (P3)
Follow-up to audit 3 §E.19.
- **Monaco:** items are matched by object identity, so a new document object gets a fresh view model and no state (`multiDiffEditorViewModel.ts:90-94`; `virtualizedItemManager.ts:105-110`).
- **Port:** a key match with changed models saves the view state (`multi_diff_editor.rs:794-800`, `:817-819`, `:1065-1067`) and restores it onto the new models (`:1007-1009`), revealed regions included.
- **Repro:** reveal some unchanged lines, then `set_documents` with the same label and new models with different text: the old revealed ranges are applied to the new text.
- **Fix:** when a kept file's models change, keep `collapsed` and the selections but drop `view_state`.

#### E.25 Collapsed files in view have no diff editor (P3)
- **Monaco:** every visible item is bound, collapsed or not (`browser/widget/multiDiffEditor/virtualizedItemManager.ts:173-208`), so `getActiveControl` returns a collapsed file's editor (`multiDiffEditorWidgetImpl.ts:209-214`).
- **Port:** collapsed files are left out of `wanted` (`multi_diff_editor.rs:2689-2693`); `book/src/diff-editor.md` ("Editors") calls this "like Monaco".
- **Repro:** collapse the active file: `active_control()` becomes `None`, and `ActiveControlChanged` fires on every collapse and expand.
- **Fix:** bind collapsed visible files with the editor hidden, or correct the book.

#### E.26 An unmeasured file is estimated at 540px, not 500px (P3)
- **Monaco:** `expandedContentHeight` starts at 500 for the whole item, header and padding included (`multiDiffEditorViewModel.ts:156-159`; content `500 - outer`, `diffEditorItemTemplate.ts:409-411`); the vendored test expects `firstObservedLastItemHeight: 500`.
- **Port:** `INITIAL_CONTENT_HEIGHT = 500` for the editor alone, with header and padding added (`multi_diff_editor.rs:61-63`, `:363`, `:419`).
- **Repro:** 20 files, `reveal(10)` before any is measured: Monaco's scroll top is 5000, the port's 5400.
- **Fix:** start the content at `500 - header - padding`.

#### E.27 The default variant can't collapse a file from the keyboard (P3)
- **Monaco:** the chevron is a focusable `Button` in every variant, toggled by Space or Enter (`diffEditorItemTemplate.ts:179-189`).
- **Port:** the chevron has no focus handle or key handler (`multi_diff_editor.rs:2197-2234`); only compact headers take focus (`:2015-2043`).
- **Repro:** in `NoCardsNonCompact`, Tab never reaches a chevron.
- **Fix:** a focus handle and an Enter/Space handler on the chevron.

#### E.28 No `focus()` equivalent, and the R flag is too eager (P3)
- **Monaco:** `focus()` focuses the active file's editor or placeholder and returns whether it could (`multiDiffEditorWidgetImpl.ts:582-590`). R shows only when the two URIs' paths differ (`diffEditorItemTemplate.ts:423-425`).
- **Port:** `Focusable` focuses only the list (`multi_diff_editor.rs:665-669`), so keystrokes go nowhere. R shows whenever `original_label.is_some()` (`multi_diff_editor.rs:376`), even when it equals `label`.
- **Fix:** add `focus_active(window, cx) -> bool`; compare the labels for R.

#### E.29 The default variant draws a sticky header shadow Monaco never shows (P3)
Decide and document (#431).
- **Monaco:** `shadow` is toggled on `.header` (`diffEditorItemTemplate.ts:504`), but the standard rule targets `.header-content.shadow` (`browser/widget/multiDiffEditor/style.css:55-62`), so it never matches; only the cards rule applies (`style.css:476-482`).
- **Port:** the default variant adds `header_shadow` whenever a file is stuck (`multi_diff_editor.rs:2005-2007`).
- **Repro:** scroll a file under its header in the default variant: no shadow in Monaco, a 6px shadow in the port.
- **Fix:** drop it outside the cards variant, or keep it and list it in `differences.md`.

## F. Options, themes and languages

Checked and matching: the `EditorOption` list (every option exists, renamed, or documented as not applicable); the standalone defaults from `editor.api.ts`; the nested defaults spot-checked (hover, sticky scroll, find, guides, bracket pair colorization); `validated()` against each `validate`; `computed_options()`; all four built-in themes' token rules (a script diff of `standalone/common/themes.ts`) and colors; every editor-side `registerColor` id in `theme/color_registry.rs`; `TokenTheme` parsing; `defineTheme`/`setTheme`; semantic token styling for standalone themes; `CharacterPairSupport`, `OnEnterSupport` and `combineLanguageConfigurations`; plaintext registration; file association guessing; `monarchCompile.ts` and `monarchLexer.ts`. The audit 3 §F.1–§F.13, §G.7 and §G.8 fixes hold, except §F.5 (see §F.4).

### Layout changes (#420)

#### F.1 Changing wrapping, font size, zoom or the editor's width doesn't keep the first visible line (P2)
- **Monaco:** `ViewModel._onConfigurationChanged` runs on every configuration change, and the computed `layoutInfo`, `fontInfo` and `wrappingInfo` count, so this covers resizes and zoom. It captures the viewport start (first visible model position and `startLineDelta`) with `_captureStableViewport`, re-wraps with `setWrappingSettings`, and calls `stableViewport.recoverViewportStart` (`common/viewModel/viewModelImpl.ts:152-158`, `:265-311`; `StableViewport` `:1465-1480`). Changing hidden areas does the same (`:604-623`).
- **Port:** `update_options` (`editor.rs:1474-1550`) swaps the options and notifies with no anchor; the next `sync_view_lines` re-wraps with `set_wrap` (`editor.rs:4960-4962`) while `scroll_top` keeps its pixel value. `keep_scroll_stable` (`geometry.rs:234-262`), the capture-and-recover, runs only for diff editor realignment (`diff_editor.rs:3860`). Zoom and window resizes take the same path.
- **Repro:** a 2,000-line file of long lines scrolled so line 1000 is at the top, then `update_options` with `word_wrap: On`: Monaco keeps line 1000 at the top; the port shows an earlier line. Zoom in at line 1000 without wrapping: the port's top line becomes about 909 (18px to 19.8px). Resizing a wrapped editor drifts the same way.
- **Fix:** call `keep_scroll_stable()` (not at `scroll_top == 0`, as Monaco) whenever the wrap settings, line height or font change: in `update_options`, on zoom, and before `set_wrap` when only the width changed. Do the same in `set_hidden_areas` unless the anchor line becomes hidden.

#### F.2 Zoomed line heights are rounded to a whole pixel; Monaco keeps them fractional (P3)
Found by the options audit (2) and the rendering audit (8).
- **Monaco:** `BareFontInfo._create` rounds the line height, then multiplies font size and line height by `1 + zoomLevel * 0.1` without rounding again (`common/config/fontInfo.ts:42-50`). At zoom level 1, 18px becomes 19.8px, and `getOption(EditorOption.lineHeight)` returns 19.8.
- **Port:** `zoomed_options` stores `resolved_line_height() * multiplier` in `line_height` (`font_zoom.rs:51-59`); everything reads it back through `resolved_line_height()`, which rounds again (`options.rs:2421`; `element.rs:267-268`), and `computed_options()` reports the rounded value (`editor.rs:8136`). The 8px minimum after zooming out is port-only; a code comment explains it, `differences.md` doesn't.
- **Repro:** macOS defaults (12px font, 18px lines), zoom in once: Monaco 19.8px lines, the port 20px, so line 50 is 10px lower and after 100 lines 20px; `computed_options().line_height` is 20. Fractional trackpad zoom levels (`browser/controller/mouseHandler.ts:182`) hit this on almost every step.
- **Fix:** don't round an already-zoomed line height again (for example a separate `zoomed_line_height` that `resolved_line_height` leaves alone), or document the rounding and the 8px minimum.

### Tokenization and language configuration (#428, #431)

#### F.3 `max_tokenization_line_length` reaches neither Monarch's tokenizer nor Monaco's exemption for `setTokensProvider` (P3)
- **Monaco:** `MonarchTokenizer` reads `editor.maxTokenizationLineLength` and returns `nullTokenize(languageId, lineState)` for longer lines, passing the state through (`standalone/common/monarch/monarchLexer.ts:430-438`, `:479-482`). `TokenizationSupportAdapter` (`setTokensProvider`, `registerTokensProviderFactory`) has no limit (`standalone/browser/standaloneLanguages.ts:190-207`).
- **Port:** `ModelOptions::max_tokenization_line_length` (`model.rs:378-381`) only filters what's displayed and queried (`element.rs:1676-1680`, `model.rs:1160-1168`, `:1204-1209`). The Monarch highlighter always uses `DEFAULT_MAX_TOKENIZATION_LINE_LENGTH` (20,000; `crates/syntax/src/monarch/mod.rs:200-216`, `:262`), and `with_max_tokenization_line_length` is called only in tests. The display filter also applies to `TokensProviderHighlighter` languages.
- **Repro:** limit 100, a C-like Monarch grammar, line 1 of 200 characters ending in an unterminated `/*`, line 2 `int x;`: Monaco skips line 1 with its state unchanged and highlights line 2 as code; the port colors line 2 as a comment. Limit 50,000 and a 30,000-character Monarch line: Monaco highlights it, the port doesn't. With `set_tokens_provider`, a 25,000-character line is highlighted in Monaco and plain in the port, and auto-closing `notIn` sees `Other` there.
- **Fix:** give the model's Monarch highlighter the model's limit, recreating it on change; exempt `TokensProviderHighlighter` from `tokenized_ranges` and `highlight_name_at`, or document both.

#### F.4 Bracket pair colorization still uses prefix checks for strings and comments; regex tokens aren't skipped (P3)
Incomplete fix of audit 3 §F.5.
- **Monaco:** the bracket pair tokenizer counts a bracket only when its token's standard type is `Other` (`common/model/bracketPairsTextModelPart/bracketPairsTree/tokenizer.ts:203-208`); the type comes from `toStandardTokenType`'s `\b(comment|string|regex|regexp)\b` (`common/languages/supports/tokenization.ts:249`), so brackets in `regexp.js`, `invalid.string` or `meta.comment` tokens are skipped.
- **Port:** §F.5 moved `token_type_at` and `string_and_comment_ranges` onto `standard_token_type` (`model.rs:97-110`, `:1216-1218`), but bracket colorization and the bracket pair guides still skip with `starts_with("comment" | "string" | "character")` (`bracket_colors.rs:104-111`), so Monarch `regexp…` tokens and `invalid.string` aren't skipped. Inline completions' `closeBrackets` has the same list plus `contains("regex")` (`inline_completions/controller.rs:3461-3466`).
- **Repro:** a Monarch rule `[/\/[^\/]*\//, 'regexp']`, brackets `()`, text `x = /(/; f(1)`: Monaco leaves the regex's `(` uncolored and gives `f(1)` the first color; the port pairs the regex's `(` with `f(1)`'s `)` and flags the `(` after `f`.
- **Fix:** filter with `standard_token_type(&name) != Other` in both places; optionally skip lines past `max_tokenization_line_length`, which Monaco treats as `Other`.

#### F.5 Tree-sitter injected languages keep their old configuration after `set_language_configuration` or re-registration (P3)
- **Monaco:** configuration is looked up by language id at each use (e.g. `getLanguageConfiguration(model.getLanguageIdAtPosition(...)).comments` in the line comment command), so a change applies at once, embedded regions included; the registry change event drives it (`common/languages/languageConfigurationRegistry.ts`, `register` → `onDidChange`).
- **Port:** `refresh_configuration` refreshes only models whose own language changed (`languages.rs:77-90`). For injections, `TextModel::language_at` returns the layer's stored `Arc<Language>` (`model.rs:1260-1273`), and `reparse` keeps cached injections for layers an edit doesn't touch (`crates/syntax/src/syntax_map.rs:299-317`; `update_root_injections` `:749-752`). The comment commands (`editor.rs:4318-4335`) and per-run word patterns (`model.rs:1287-1350`) read that stale copy. Monarch `nextEmbedded` regions look the language up fresh (`model.rs:1248-1258`).
- **Repro:** an HTML model with `<script>\nlet x;\n</script>`, then `set_language_configuration(cx, "javascript", { comments: { lineComment: "#" } })`; Cmd+/ on `let x;`: Monaco-style lookup gives `# let x;`, the port `// let x;` until the script block is edited.
- **Fix:** re-resolve each layer's language by id in `language_at` and `line_language_runs`, or invalidate cached injections in `refresh_configuration` and `languages_changed`.

#### F.6 `register_tokens_provider_factory` doesn't replace an earlier factory for the same language (P3)
- **Monaco:** `TokenizationRegistry.registerFactory` disposes the language's existing factory first (`common/tokenizationRegistry.ts:49-58`); only the latest is resolved, and disposing the old one removes a tokenizer it installed (`:115-151`).
- **Port:** each call queues its own `create` callback (`languages.rs:208-262`, `when_requested` `:246`); both are called when the language is requested (or at once if it was), both install, and the last to finish wins.
- **Repro:** `register_tokens_provider_factory(cx, "x", f1)` then `(cx, "x", f2)`, `f1` resolving after `f2`, then open a model in `x`: Monaco calls only `f2`; the port calls both and ends up with `f1`'s tokenizer.
- **Fix:** one factory slot per language id; a new registration disposes the previous one (its waiting callback and any tokenizer it installed).

#### F.7 Disposing a tokenizer restores the one it replaced; Monaco leaves the language untokenized (P3)
Decide and document (#431).
- **Monaco:** `TokenizationRegistry.register` keeps one support per language; its disposable deletes the entry if still current and doesn't bring back what it replaced (`common/tokenizationRegistry.ts:33-43`).
- **Port:** `install_highlighter` remembers the previous highlighter and restores it on dispose (`languages.rs:272-290`), for `set_monarch_tokens_provider`, `set_tokens_provider` and factories. The doc comments say so; `differences.md` doesn't.
- **Repro:** `let a = set_monarch_tokens_provider(cx, "x", &A)?; let b = set_monarch_tokens_provider(cx, "x", &B)?; b.dispose(cx);`: Monaco's models in `x` lose highlighting; the port's go back to A.
- **Fix:** restore only a language's own registered grammar, not an earlier runtime tokenizer, or list the behavior in `differences.md`.

## G. Public API

Scope: `monaco.editor.*`, `ICodeEditor`/`IStandaloneCodeEditor`, `ITextModel`, `IDiffEditor`, `monaco.languages.*` registration and provider interfaces, `Range`/`Position`/`Selection`, `KeyCode`/`KeyMod`, and `book/src/migrating.md`. Paths starting `crates/gpui-monaco-lsp/` are the LSP bridge.

Checked and matching, including the audit 3 §G.1–§G.24 fixes: `set_selections_from` without a reveal for `Api`, `CursorChangeSource::Other`, `computed_options`, `id`/`editor_type`, `get_action`, action descriptors, `DecorationsChanged`, custom line-height decorations, `DecorationFilter`, `inmemory://model/N` resources, markers by uri with the 500 cap, MIME types in `colorize`/`set_language`, `normalize_uri`, `LanguageChanged.source`, `apply_edit_operations`, `validate_range` around surrogates, `word_at_position` per embedded language, `define_theme`/`set_theme`, opener order, `set_language_configuration` for unknown ids, `on_language`/`on_language_encountered`, `add_keybinding_rules` with `command: None`, `MarkerService.read`/`take`, `DecorationsCollection.on_did_change`, read-only `execute_edit_operations_from`, `LanguageSelector` scoring, every function `migrating.md` names, the `IDiffEditor` surface, and provider consumption (completion groups and early stop, trigger characters, incomplete lists, resolve, signature help retrigger rules, code action filtering and sorting, document highlights, code lens order, definition merging).

### Go to References (#401)

#### G.1 Go to References lacks Monaco's "compact" re-query (P2)
- **Monaco:** `GoToReferencesAction` calls `getReferencesAtPosition(…, compact=true, …)` (`contrib/gotoSymbol/browser/goToCommands.ts:698`); when a provider returns exactly 2 references, Monaco asks again with `includeDeclaration: false` and uses that answer if it has exactly 1 (`contrib/gotoSymbol/browser/goToSymbol.ts:77-89`). Peek References passes `compact=false` (`goToCommands.ts:726`).
- **Port:** every reference request uses `include_declaration: true` once (`goto.rs:163-174`); Go to and Peek References share `navigate` (`goto.rs:313-350`).
- **Repro:** `let x = 1; use(x);`, cursor on the declaration, Shift+F12: Monaco jumps to `use(x)`; the port opens a peek with both.
- **Fix:** a `compact` flag for `GoToReferences` that repeats the request per provider with `include_declaration: false` when the first answer has 2 items.

### Model language (#409)

#### G.2 Models without a language aren't `plaintext` to provider selectors, providers or `editorLangId` (P2)
- **Monaco:** a model with no or an unregistered language has `getLanguageId() === 'plaintext'` (`_createAndGetLanguageIdentifier`, `common/services/languageService.ts:124-131`). Selectors score against it and the model's uri, `inmemory://model/N` when none is given (`common/languageFeatureRegistry.ts:163-181`, `common/model/textModel.ts:321-322`), so `registerXProvider('plaintext', …)` and `{ scheme: 'inmemory' }` apply, and `editorLangId == plaintext` holds.
- **Port:** `resolve_language(None)` returns `(None, plain_text)` (`model.rs:70-75`), so `language_id()` is `None`. `ProviderTarget::from(&TextModel)` uses `language_id()` and `uri()`, not `resource()` (`providers.rs:114-120`); `ProviderContext.language_id` is `None` (`model.rs:816-821`); `editorLangId` isn't set (`context_keys.rs:271-273`). `LanguageSelector::matches` documents that such models match only `All`/`*` (`providers.rs:129-131`); `languages.md:20` documents the unregistered-id case, not `None`.
- **Repro:** `register_hover_provider(cx, LanguageSelector::language("plaintext"), p)`, show `create_model(cx, "hi", None, None)`, hover: Monaco asks `p`, the port never does. A `LanguageFilter { scheme: Some("inmemory".into()), .. }` doesn't match, and `editorLangId == plaintext` fails.
- **Fix:** use the resolved language's id (`language().id`, `plaintext`) and `resource()` in `ProviderTarget`, `ProviderContext` and the context keys.

#### G.3 A model's language is resolved only when an editor first shows it (P2)
- **Monaco:** `createModel` resolves the language at once, guessing from the uri or first line (`standalone/browser/standaloneCodeEditor.ts:584-595`), and the `TextModel` constructor calls `requestRichLanguageFeatures` (`common/model/textModel.ts:406`), so `onLanguage` fires at `createModel`. Registering providers in `onLanguage` and then using the model headless is common.
- **Port:** `with_uri`/`with_language` only set `language_pending` (`model.rs:763-787`). Resolution, the uri guess and `language_requested` (which fires `on_language`) happen in `ensure_syntax` (`model.rs:1036-1056`), run only from `CodeEditor::new`/`set_model` (`editor.rs:1065-1068`, `:1334-1337`) and `set_language`; not from `create_model` (`model.rs:265-288`), `register_model` (`model.rs:196-210`) or `api_commands::target` (`api_commands.rs:23-26`). `languages.md:97` says the guess happens "when it's first shown, as Monaco's `createModel` does".
- **Repro:** `create_model(cx, "fn main() {}", None, Some("file:///a.rs"))?` then `language_id()`: Monaco `rust`, the port `None`. `api_commands::execute_link_provider(&m, 0, cx)` with a `rust` link provider returns nothing. `on_language(cx, "rust", …)` doesn't fire for `create_model(…, Some("rust"), None)` until shown, and `on_did_create_model` listeners see no language.
- **Fix:** call `ensure_syntax` in `create_model`/`register_model` (or resolve and request eagerly, keeping only the parse lazy), and fix `languages.md`.

#### G.4 A language name, alias or extension gives the grammar but stays the model's id (P3)
- **Monaco:** `createModel`/`setModelLanguage` accept a language id or a MIME type; anything else is `plaintext` (`standalone/browser/standaloneEditor.ts:225-244`, `languageService.ts:124-131`).
- **Port:** `resolve_language` also tries `language_for_name` (`model.rs:77-81`), matching id, name, aliases or extensions case-insensitively (`crates/syntax/src/language.rs:545-557`, `:364-369`), and keeps the given string as `language_id`.
- **Repro:** `TextModel::new(src).with_language("Rust")` (or `"rs"`, `"RUST"`): highlighted as Rust, but `language_id()` is `"Rust"`, so `rust` providers, `editorLangId == rust` and `set_language_configuration(cx, "rust", …)` don't apply. Monaco gives a consistent `plaintext`.
- **Fix:** store the canonical id when a name matches (as the MIME branch does), or drop the name fallback for models.

### Commands and keybindings (#412)

#### G.5 Commands on completion items and code actions can't run editor actions (P2)
- **Monaco:** after inserting an item, its `command` goes through `ICommandService.executeCommand` (`contrib/suggest/browser/suggestController.ts:459-466`), where editor actions are commands, so `editor.action.triggerParameterHints` (common on function completions) and `editor.action.rename` work.
- **Port:** `accept_suggestion_with` handles only the next-placeholder command and `editor.action.triggerSuggest`; anything else goes to the global `commands::execute_command`, and a failure is only logged (`completion.rs:2379-2387`). The editor-action resolution in `CodeEditor::run_command` (`editor_actions.rs:1273-1327`) is never tried, and there's no `ExecuteCommand` fallback to the host, which code lenses have (`code_lens.rs:575-578`). Code action commands are the same; the error is shown as a message (`code_action.rs:1696-1699`).
- **Repro:** accept `CompletionItem { insert_text: "foo($0)", command: Some(Command::new("editor.action.triggerParameterHints", "")), .. }`: Monaco opens parameter hints; the port logs "command … not found".
- **Fix:** route provider commands through `run_command` (deferred to get a `Window`), falling back to `CodeEditorEvent::ExecuteCommand`.

#### G.6 Keybinding rules can't remove a default binding with `-command` (P2)
Upstream, not vendored.
- **Monaco:** `addKeybindingRule(s)` adds dynamic keybindings as overrides (`standalone/browser/standaloneServices.ts:425-446`, `:466-494`). The resolver (`KeybindingResolver`, `vs/platform`) treats a `-`-prefixed command as removing that command's default binding, matching the keybinding (if any) and `when`; other commands on the key keep it. `monaco.d.ts:1013-1018` types `command` as `string | null`. `{ keybinding: KeyMod.CtrlCmd | KeyCode.KeyF, command: '-actions.find' }` is the usual way to free a default key.
- **Port:** `add_keybinding_rules` turns every `Some(command)` into a `RunCommand { id }` binding (`editor_actions.rs:947-973`), so `-actions.find` binds a command that doesn't exist (`run_command` returns `false`, `editor_actions.rs:1273-1327`) and shadows every binding of the keystrokes in the editor context. A rule with no keystrokes, which removes all the command's bindings, fails to parse.
- **Repro:** `add_keybinding_rule(cx, KeybindingRule::new("cmd-f", "-actions.find"))`: in Monaco Cmd+F no longer opens find and other Cmd+F bindings still apply; in the port Cmd+F runs nothing.
- **Fix:** for a `-` command, remove the matching default bindings for that action (and `when`) the way `bind_removable` does, or document it as unsupported in `keymap.md`/`differences.md`.

### Provider changes (#416)

#### G.7 Links aren't recomputed when the language or the link providers change (P2)
- **Monaco:** `LinkDetector` recomputes on `onDidChangeModelLanguage` and on `linkProvider.onDidChange`, which fires on register and unregister (`contrib/links/browser/links.ts:103-110`).
- **Port:** `refresh_links` recomputes only on a buffer version change (`links.rs:116-131`); the `LanguageChanged` handler resets inlay hints and folding but not links (`editor.rs:2987-3007`); `register_link_provider` and `unregister_provider` don't refresh links (`providers.rs:3124-3130`, `:3211-3249`).
- **Repro:** register a link provider for `foo`, then `set_language(Some("foo".into()))`: Monaco underlines its links at once; the port shows only the built-in URL links until the next edit plus the 1s delay. An unregistered provider's links stay clickable.
- **Fix:** add the language id and a provider-registry epoch to the links key, as `color_detector.rs` does.

#### G.8 Inlay hints and code lenses ignore providers registered or removed later (P3)
Found by the API audit (11) and the widgets audit (35).
- **Monaco:** `inlayHintsProvider.onDidChange` calls `_update()`, which clears the hints and asks again, or stops with none when no provider is left (`contrib/inlayHints/browser/inlayHintsController.ts:161`, `:179-189`). Code lenses do the same.
- **Port:** hints are asked again only for an edit, a `refresh_inlay_hints` epoch, a reset or uncovered rows (`inlay_hints.rs:478-492`); `inlay_hints_viewport` and `inlay_hints_after_edit` are gated on active hints and coverage, so `schedule_inlay_hints`, where clearing happens (`:218-224`), never runs. Registering or unregistering doesn't bump the epoch, and `code_lens.rs:337-360` reacts only to its own epoch.
- **Repro:** with provider A's hints showing, `unregister_provider(cx, a)`: Monaco removes them; the port keeps them, still clickable and resolvable, through edits and scrolls in covered rows. Registering B doesn't show B's hints until an edit or scroll.
- **Fix:** bump the inlay hint and code lens epochs in their `register_*` and in `unregister_provider`, and clear the items when `!inlay_hints_active(cx)` and some remain.

### Provider requests (#417)

#### G.9 Formatters get `tab_size` where Monaco passes `indentSize` (P2)
- **Monaco:** `getFormattingOptions()` returns `{ tabSize: this._options.indentSize, insertSpaces }` (`common/model/textModel.ts:682-687`), for document, range, ranges, on-type and paste formatting.
- **Port:** `formatting_options` sends the model's `tab_size` (`format.rs:200-206`); `indent_size` (`model.rs:370`) is never sent. The field's doc says "Spaces per indentation level (and per tab)" (`providers.rs:2419-2420`).
- **Repro:** model options `tab_size: 8, indent_size: 2, insert_spaces: true`, Format Document with a provider indenting by `options.tab_size`: Monaco indents by 2, the port by 8.
- **Fix:** send the resolved indent size (`tab_size` when `indent_size_is_tab_size`) and fix the field doc.

#### G.10 Semantic tokens: lower-ranked providers are used, and range providers get no fallback (P3)
- **Monaco:** document tokens come only from the top-score group (`orderedGroups(model)[0]`): the first result with tokens, else the first result. Range tokens ask every provider in the top group (`contrib/semanticTokens/common/getSemanticTokens.ts:66-88`, `:101-139`).
- **Port:** `fetch_document_semantic_tokens` asks every matching provider across scores and takes the first non-`None` (`semantic_tokens/mod.rs:227-275`); the viewport path asks only the single best range provider (`.next()?`, `semantic_tokens/mod.rs:431-434`).
- **Repro:** provider A for `rust` returns `None`, B for `*` returns tokens: Monaco shows none, the port colors with B. Two equal-score range providers where the newest returns `None`: Monaco uses the other's tokens, the port shows none.
- **Fix:** group by score and keep the top group for both paths, taking the first result with tokens.

#### G.11 Completion items with an invalid `replace_range` aren't rejected (P3)
- **Monaco:** an item whose `{insert, replace}` replace range spans lines, isn't on the cursor's line, or doesn't start at the insert range's start is invalid and skipped (`contrib/suggest/browser/suggest.ts:113-120`, `completionModel.ts:146`).
- **Port:** `prepare` checks only `item.range` (`suggest/completion_model.rs:350-363`); the doc says `replace_range` "must start at the same position", but nothing checks it (`providers.rs:479-482`).
- **Repro:** `range: (0,4)..(0,6)`, `replace_range: Some((0,2)..(0,8))`: Monaco hides it; the port lists it, and replace mode overwrites from column 2.
- **Fix:** treat such items as invalid in `prepare`.

#### G.12 The LSP bridge drops `activeSignatureHelp` and gets `isRetrigger` wrong (P3)
- **Monaco:** `ParameterHintsModel.doTrigger` sends `isRetrigger` (hints showing or pending) and `activeSignatureHelp` with the user's overload (`contrib/parameterHints/browser/parameterHintsModel.ts:178-204`); the core port passes both in `SignatureHelpContext`.
- **Port:** `LspFeatures` doesn't override `provide_signature_help_with_context` (default at `providers.rs:2355`); it sends `active_signature_help: None` and derives `is_retrigger` from the character alone (`crates/gpui-monaco-lsp/src/features.rs:533-551`) while advertising `contextSupport: true` (`crates/gpui-monaco-lsp/src/client.rs:775-781`).
- **Repro:** with a language server, pick overload 2/3 with Alt+Down, type `,`: the server gets `activeSignatureHelp: null` and `isRetrigger: false` and goes back to 1/3.
- **Fix:** implement `provide_signature_help_with_context` and map both fields.

### Hover, links and inline completions (#405, #415, #430)

#### G.13 A provider hover without a `range` gets no hover highlight (P3)
- **Monaco:** the standalone `registerHoverProvider` wrapper defaults `Hover.range` to the word at the position, else an empty range (`standalone/browser/standaloneLanguages.ts:490-512`); `hoverHighlight` covers the union of the parts' ranges (`contrib/hover/browser/markdownHoverParticipant.ts:188`, `contentHoverRendered.ts:255-267`).
- **Port:** such a part gets `position..position` (`hover.rs:2170`, `:1975`), so the union is empty and `hover_highlight_range` drops it (`hover.rs:2816-2838`); the word is used only for the keep-open range (`hover.rs:2145-2147`).
- **Repro:** `Hover { contents: vec!["doc".into()], range: None }`, hover `foo`: Monaco shades it with `editor.hoverHighlightBackground`, the port doesn't.
- **Fix:** default the part range to the word range computed at `hover.rs:2145`.

#### G.14 `api_commands::execute_link_provider` doesn't merge, sort or include the built-in links (P3)
- **Monaco:** `_executeLinkProvider` returns `getLinks(…).links` (`contrib/links/browser/getLinks.ts:171-196`), which asks `ordered(model).reverse()` and unions the lists, so better-ranked providers replace overlapping links and the result is sorted (`getLinks.ts:79-139`, `:143-150`). The built-in URL detector is a `*` link provider (`browser/services/editorWorkerService.ts:87`), so its links are included.
- **Port:** asks providers best first and concatenates in provider order, with no union, sort or computed links (`api_commands.rs:30-60`). The editor's own link path does all three (`links.rs:141-189`).
- **Repro:** `https://a.example` in a comment and two providers with overlapping ranges: Monaco returns the URL link and the better provider's link, sorted; the port both providers' links and no URL link.
- **Fix:** share the union and link-computer path from `links.rs`.

#### G.15 Detected links never open by default, unlike markdown links (P3)
Upstream opener, not vendored.
- **Monaco:** `openLinkOccurrence` hands the link to `IOpenerService.open(…, { allowCommands: true, … })` (`contrib/links/browser/links.ts:221-260`); with no registered opener taking it, the default opener opens `http(s)`/`mailto` in the browser and runs `command:` links.
- **Port:** a detected link no `register_link_opener` opener takes only emits `CodeEditorEvent::OpenLink` (`openers.rs:104-119`); `providers.md:420` says "the editor never opens anything itself". Markdown links in hovers do fall back to the browser and trusted `command:` (`markdown.rs:818-830`, `model-and-view.md:163`).
- **Repro:** no opener and no `OpenLink` subscriber, Cmd+click `https://example.com` in a comment: nothing happens. The same URL in a hover opens the browser, as Monaco does in both cases.
- **Fix:** after the openers, and if nobody handles `OpenLink`, apply the markdown fallback (`is_openable`, `cx.open_url`), or list the difference in `differences.md`.

#### G.16 `InlineCompletionList.commands` can't carry an icon (P3)
- **Monaco:** list commands are `InlineCompletionCommand { command, icon? }` (`common/languages.ts:943`), drawn with `c.icon ?? Codicon.symbolEvent` (`contrib/inlineCompletions/browser/view/inlineEdits/components/gutterIndicatorMenu.ts:69-75`).
- **Port:** `commands: Vec<Command>` (`providers.rs:1823`); the menu always draws `Codicon::SymbolEvent` (`inline_completions/inline_edits_view.rs:1166-1171`).
- **Repro:** a command with a sparkle icon: Monaco shows the sparkle, the port the symbol-event icon.
- **Fix:** add `icon: Option<Codicon>` per command.

### Migration guide (#431)

#### G.17 `migrating.md` gaps (P3)
- **`KeyMod`/`KeyCode`:** no mapping. `KeyMod.CtrlCmd` is GPUI's `secondary-` (Cmd on macOS, Ctrl elsewhere), `KeyMod.WinCtrl` is `ctrl-`, and `KeyMod.chord(a, b)` is a space-separated `"cmd-k cmd-m"`. Neither `migrating.md` nor `keymap.md` mentions `secondary-`, so ported rules tend to hard-code `cmd-`; `KeybindingRule.keystrokes` says only "GPUI syntax" (`editor_actions.rs:887-889`).
- **`Range`/`Position`/`Selection` helpers:** Monaco's `containsPosition`, `containsRange` and `areIntersectingOrTouching` are end-inclusive, Rust's `Range::contains` isn't, so a direct translation misses a position at the end. Only `PointUtf16::from_monaco`/`to_monaco` are mentioned (`migrating.md:6`).
- **Missing rows:** `monaco.editor.createDiffEditor`/`IDiffEditor` (`DiffEditor::new`, `line_changes`, `go_to_diff`…, only in `diff-editor.md`), `createMultiFileDiffEditor`, `languages.getLanguages` (`LanguageRegistry::languages`, `crates/syntax/src/language.rs:526`) and `registerNewSymbolNameProvider` (`register_new_symbol_name_provider`, `providers.rs:3156`; the language features table at `migrating.md:178-199` skips it).
- **`migrating.md:16`** maps `monaco.editor.create(el, { value, language })` to an unregistered `TextModel`. Monaco's `create` model is in the model service, so `getModel(uri)`/`getModels()` and word-based suggestions see it; `model_for_uri` only sees `register_model`ed models (`model.rs:196-210`). Use `create_model` there.
- **`languages.md:97`** claims the uri guess happens on first show "as Monaco's `createModel` does" (§G.3).

## H. Input, accessibility and keybindings

Scope: `actions.rs` `default_key_bindings` and `by_precedence` (all three platform blocks) against `browser/coreCommands.ts` and every `kbOpts`/`KeybindingsRegistry` rule under `contrib/**`, `standalone/browser/**` and `browser/widget/diffEditor/commands.ts`; `editor_actions.rs` preconditions; `context_keys.rs` against `common/editorContextKeys.ts`; mouse handling against `browser/controller/mouseHandler.ts` and `browser/view/viewController.ts`; `dnd.rs`; the IME handler; `accessibility.rs` against `screenReaderUtils.ts` and `textAreaEditContext.ts`; tab focus mode, blur handling and every `alert`/`status` call.

Checked and matching: the per-platform key tables rule for rule, apart from §H.2 and §H.5. Audit 3 §H.1–§H.15 are fixed with no regressions (contextual bindings after plain ones, `TAB_OUTWEIGHS_FOCUS`, `TAB_MOVES_FOCUS` with `editorReadonly`, the Escape chain in `editor.rs:3829-3906`, `menu`, `composition_type_at_cursors`, paged view-line content, and every vendored `alert`/`status` site with a counterpart, `announce_status` polite).

### Widget accessibility (#413)

#### H.1 Editor widgets expose no accessible names or roles: find, replace, rename and quick input are "Editor content", and menus and lists are silent (P2)
- **Monaco:** the find widget is `role="dialog"` labelled "Find / Replace" (`contrib/find/browser/findWidget.ts:1260-1261`), its inputs "Find" and "Replace" (`:65`, `:71`, `:1340-1361`), its buttons `role="button"` with labels such as "Previous Match", "Next Match", "Close" (`:1420-1421`, labels `:67-75`). The rename input's label is "Rename input. Type new name and press Enter to commit." (`contrib/rename/browser/renameWidget.ts:926`). The references peek tree is labelled "References", and each row's label is its `ariaMessage`, e.g. "{0} in {1} on line {2} at column {3}" or "{n} symbols in {file}" (`contrib/gotoSymbol/browser/peek/referencesTree.ts:215-223`, `referencesModel.ts:48-60`, `:119-125`). Sticky scroll lines are a `role="list"` of `listitem`s (`contrib/stickyScroll/browser/stickyScrollWidget.ts:82-85`, `:490`). The context menu, quick input list and code action list are `menu`/`menuitem` and `listbox`/`option` widgets with `aria-activedescendant` (upstream `base/browser/ui/menu`, `platform/quickinput`, `platform/actionWidget`, not vendored).
- **Port:** every widget input is a `CodeEditor::new_widget_input` (`editor.rs:938-950`) with `widget_input_options()` (`editor.rs:8158-8213`), which never set `aria_label`, so the inputs keep the default "Editor content" (`options.rs:2234`) that the root node uses as its label (`editor.rs:7126-7130`): the find and replace inputs (`find_widget.rs:398-416`, a placeholder only), rename (`rename.rs:122`) and quick input (`quick_access.rs:235`). The find controls have tooltips but no `.role()` or `.aria_label()` (`find_widget.rs:171-185`, `:1194-1223`, `:1246-1270`), and the widget has no dialog role. `.role(`/`.aria_label(` calls cover only the editor root, hover, markers, the unicode banner and the diff editor; `context_menu.rs`, `quick_access.rs`, `code_action.rs`, `peek.rs` and sticky scroll have none, and `peek.rs` doesn't announce focus changes. The context menu (`context_menu.rs:575`), code action menu and peek list take focus, so a screen reader lands on unnamed nodes.
- **Repro:** VoiceOver, NVDA or the AccessKit inspector. Cmd/Ctrl+F reads "Editor content, edit text" instead of "Find", and Tab to the next button reads nothing where Monaco reads "Next Match (Enter), button". F2 reads "Editor content". Shift+F12 then Down: Monaco reads "foo.rs on line 12 at column 5", the port nothing. Shift+F10 or Cmd+. and arrowing: nothing.
- **Fix:** a label parameter on `new_widget_input` ("Find", "Replace", the rename text, the quick input placeholder); `.role(Button/CheckBox)` and `.aria_label(title)` on find controls and `Role::Dialog` on the widget; `Role::Menu`/`MenuItem` and `Role::ListBox`/`ListBoxOption` with labels and `set_active_descendant` on menus and lists; `Role::Tree`/`TreeItem` with Monaco's `ariaMessage` on the peek list.

#### H.2 The editor node lacks Monaco's `aria-roledescription="editor"` and `aria-autocomplete` (P3)
- **Monaco:** the textarea gets `role="textbox"`, `aria-roledescription` "editor" (localized), `aria-multiline="true"` and `aria-autocomplete` "both", or "none" when `readOnly` (`browser/controller/editContext/textArea/textAreaEditContext.ts:192-196`), so screen readers announce an "editor" with completions.
- **Port:** the root node gets a role (`MultilineTextInput`/`TextInput`) and a label (`editor.rs:7116-7130`); the synthetic-children builder adds read-only, required and active descendant, but no role description or auto-complete (`editor.rs:7144-7162`).
- **Repro:** focus the editor with VoiceOver: Monaco reads "Editor content, editor", the port "Editor content, edit text".
- **Fix:** `node.set_role_description("editor")` and `node.set_auto_complete(if read_only { None } else { Both })` in the `a11y_synthetic_children` closure.

### Escape and mouse focus (#412, #411)

#### H.3 Escape hides the paste/drop options chip ahead of heavier Escape rules (P3)
- **Monaco:** `editor.hidePasteWidget` and `editor.hideDropWidget` bind Escape at weight `EditorContrib` (`contrib/dropOrPasteInto/browser/copyPasteContribution.ts:41-48`, `dropIntoEditorContribution.ts:38-42`), so `hideSuggestWidget` (+90), `closeParameterHints` (+75, `parameterHints.ts:110`), `closeMarkersNavigation` (+50, `gotoError.ts:299`), `leaveSnippet` (+30, `snippetController2.ts:385`) and `closeFindWidget` (+5, `findController.ts:1076`) run first.
- **Port:** `escape → HidePostEditWidget` is in `POST_EDIT_WIDGET` (`actions.rs:35`, `:1868`); being contextual, `by_precedence` puts it after the plain `escape → RemoveSecondaryCursors` (`actions.rs:2390-2404`), so it beats the weighted chain in `CodeEditor::escape` (`editor.rs:3829-3906`).
- **Repro:** find open, text focused, paste so the chip shows, Escape: Monaco closes find and keeps the chip; the port hides the chip and leaves find open. A provider edit that is a snippet: Monaco leaves the snippet, the port hides the chip.
- **Fix:** drop the contextual binding and call `clear_post_edit` from `CodeEditor::escape` after `close_find` (weight `EditorContrib`, with `cancel_symbol_navigation`/`cancel_operation`), for the non-secondary Escape only.

#### H.4 Clicking the minimap or scrollbars, or right-clicking outside the text, takes focus; with `context_menu: false`, right-click doesn't move the cursor on gutter targets (P3)
- **Monaco:** `MouseHandler._onMouseDown` focuses only for content targets, or line numbers with `selectOnLineNumbers`; gutter targets `preventDefault`, and scrollbar and minimap targets match no branch (`browser/controller/mouseHandler.ts:313-345`); the minimap `preventDefault`s every pointer-down (`browser/viewParts/minimap/minimap.ts:1237-1238`, `:1274-1275`). Right-click goes through `ContextMenuController._onContextMenu` (`contrib/contextmenu/browser/contextmenu.ts:84-134`), which focuses only for `CONTENT_TEXT`/`CONTENT_EMPTY`/`TEXTAREA`; with `contextmenu: false` it focuses and `setPosition(target.position)` for any target with a position, line numbers and glyph margin included (`:89-95`).
- **Port:** `on_mouse_down` calls `window.focus` for every target except the glyph margin, line decorations and (without `selectOnLineNumbers`) line numbers, before handling the minimap and scrollbars (`editor.rs:5456-5469`, then `:5476-5560`). `on_right_mouse_down` focuses for every target outside widgets (`editor.rs:5356-5362`). `context_menu_mouse_down`, the only right-click cursor move, runs only for `ContentText`/`ContentEmpty` (`editor.rs:5392-5401`, `context_menu.rs:1032-1050`).
- **Repro:** two editors, type in A, click B's minimap or scrollbar track: Monaco keeps focus in A, the port moves it to B. Right-click B's line numbers: the same. With `context_menu: false`, right-click line 30's number: Monaco puts the cursor at 30:1, the port doesn't move it.
- **Fix:** focus only for content targets (and line numbers with `select_on_line_numbers`); in `on_right_mouse_down`, focus inside the content branch; with `context_menu` off, apply "move unless inside a selection" to any target with a position.

### VS Code additions (#431)

#### H.5 Insert toggles overtype mode, a binding and palette action standalone Monaco doesn't have (P3)
Decide and document.
- **Monaco:** the vendored editor has `InputMode` (`common/inputMode.ts:10-20`) but registers no `toggleOvertypeInsertMode` action and no Insert keybinding; `KeyCode.Insert` appears only in the Windows Ctrl+Insert/Shift+Insert clipboard secondaries (`contrib/clipboard/browser/clipboard.ts:86`, `:126-127`; `contrib/suggest/browser/suggestController.ts:1128`). The command and key come from the VS Code workbench.
- **Port:** binds `insert → ToggleOvertypeInsertMode` on every platform (`actions.rs:1790`) and lists "Toggle Overtype/Insert Mode" in the palette (`editor_actions.rs:251`); neither `differences.md` nor `keymap.md` mentions it.
- **Repro:** on Windows or Linux press Insert, then type over text: the port overwrites (with the overtype cursor); Monaco keeps inserting.
- **Fix:** remove the default `insert` binding (keeping the action for hosts), or document it in `differences.md` as a VS Code addition.
