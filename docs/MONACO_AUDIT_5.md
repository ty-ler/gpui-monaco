# Monaco parity audit 5 (2026-10-12)

A fifth audit, done at `f279bde` after every issue from [`MONACO_AUDIT_4.md`](MONACO_AUDIT_4.md) (#400–#431) was fixed. Eight read-only audits compared the port with the vendored Monaco source, one per area: editing and cursors, view rendering, widgets and menus, navigation and scrolling and folding, the diff editors, options and themes and languages, the public API and the LSP bridge, and input, accessibility and keybindings. Every P1 and P2 and a sample of the P3s were checked again by reading both sides at the cited lines. Nothing was built or run, so IME, mouse, drag and screen reader findings come from reading the code.

Most audit 4 fixes hold. These were found incomplete; each follow-up is listed below:

- Audit 4 §A.4: a single-cursor IME composition still goes through `replace_range`, one undo step per update (here §A.1), and a replacement on the caret's line that doesn't contain the caret still collapses the cursors (here §A.2).
- Audit 4 §C.22: the find options flash only when find is closed (here §C.25).
- Audit 4 §C.35: code lenses are reconciled by row, but their zones don't follow their line between requests (here §C.14).
- Audit 4 §F.2: the line height is kept unrounded after zoom, but `computed_options().font_size` still clamps (here §F.9).
- Audit 4 §H.1: the suggest list got no roles (here §H.10).
- Audit 4 §H.4: the left and right buttons were fixed, the middle button wasn't (here §H.3), and with `context_menu: false` a right-click tests every selection (here §H.7).

Others follow up a fix that held: §B.3 (exposed by audit 4 §B.11 measuring `em_width` from `n`), §C.12 (audit 4 §C.1/§C.4), §C.19 (audit 4 §C.30), §C.24 (audit 4 §C.27), §E.2 (a side effect of audit 4 §E.6 keeping the whole-file region) and §E.7 (audit 4 §E.23 covers only the layout fields).

The audits overlapped in one place: the editing and widget audits both found the alphanumeric filter on the find seed and Cmd+D word (§A.6). It is listed once. Left out:

- what [`book/src/differences.md`](../book/src/differences.md) documents as a deliberate difference with a reason that holds;
- what [`MONACO_PARITY.md`](MONACO_PARITY.md) lists as not ported with a reason that holds;
- tracked work: the GPUI-blocked items (#231–#233, #305–#309), publishing to crates.io (#57) and the performance investigation (#378);
- findings already fixed on `main`, and code-style differences with no observable effect.

`vs/platform`, `monaco-languageclient` and most of `vs/base/browser` aren't vendored. Findings that rely on that code say *upstream, not vendored* and are worked from VS Code or monaco-editor 0.55.1 as they ship.

Paths: the port's are relative to `crates/gpui-monaco/src/` unless they start with `crates/`, `book/` or `docs/`. Monaco's are relative to `vendor/vscode/src/vs/editor/` unless they start with `base/`.

Severity: **P1** visibly broken or wrong behavior; **P2** a missing Monaco sub-feature or a notable fidelity gap; **P3** polish.

## Issues

| Issue | Scope | Sections |
|---|---|---|
| #469 | IME: a single-cursor composition is one undo step per update; replacements beside the caret collapse the cursors | A |
| #470 | Suggest word ranges and accepting: the language word pattern, accepting after the caret moved right deletes text, commit characters on a frozen list | C |
| #471 | LSP bridge uris: paths with spaces can't attach and normalized server uris lose diagnostics; `applyEdit`, unattached diagnostics, diagnostic codes | G |
| #472 | Widget focus vs text focus: host actions run in the find input, a focused sticky line edits the text, bracket highlights on widget focus | D, H |
| #473 | Editor commands and undo: non-keyboard `type` and `paste`, the linked-editing mirror after a paste, `cursorMove` by view columns | A |
| #474 | Hover: the character under the mouse, keeping a hover the mouse moves toward, leaving to outside the editor, focus across rebuilds, verbosity | C |
| #475 | Suggest and parameter hints: `sortText` fallback, resolved edits' undo step, shifted additional edits, provider and language changes, signature help order | C |
| #476 | Inline toolbar, color pickers, paste/drop and the message: the always-shown count, edits from elsewhere, failed resolves, re-picking, close bounds | C |
| #477 | Code lens and code actions: zones following their line, refresh and resolve timing, the lightbulb under a drag, the lone AI fix | C |
| #478 | Quick access: Go to Offset with CRLF, Go to Symbol activation, empty and busy states | C |
| #479 | Folding: commands before the first range computation, removing manual ranges, middle-clicking `⋯` | D |
| #480 | Diff editor: CRLF in staged values and copies, reveals in identical diffs, the moved-block compare decorations, toolbar overflow, viewer whitespace | E |
| #481 | Embedded languages and bracket configuration: typing rules at the cursor's language, the bracket union, empty lists, indentation bracket stripping | F |
| #482 | LSP bridge: code lens, document links, colors and linked editing | G |
| #483 | Decorations and the gutter: glyphs with the glyph margin off, gutter ranges, guide spacing, tabular line numbers, `hideIn*Tokens` | B, G |
| #484 | Token colors outside the main view: semantic tokens in sticky scroll and the minimap, injected text in the minimap, selection foreground | B |
| #485 | Find and multi-cursor: symbol-only words, sessions on focus changes, move and add-previous, Select All's limit, the find options flash, widget edge cases | A, C |
| #486 | Font options: `allowVariableFonts`, two-cell width with ligatures, the zoomed font size, ligature strings | B, F |
| #487 | Go to definition: the Cmd/Ctrl+hover link per word, the target highlight in high contrast | D |
| #488 | Multi diff editor: file action state and icons, list-wide option overrides | E |
| #489 | Language and theme definitions: JavaScript regex classes, `first_line` anchoring, `inherit: false` token color | F |
| #490 | Model uris in the API: `goToLocations` commands, `ModelChange`, `ProviderContext` | G |
| #491 | Mouse and drag-and-drop: middle clicks, read-only drops, the drag pointer, the copy modifier after blur, secondary selections, target color | H |
| #492 | Accessibility: read-only announcement, suggest list roles | H |
| #493 | Docs: smart select's syntax-tree ranges, byte ranges in `differences.md`, the LSP feature list | D, G |

Several findings share a root cause and are filed together:

- **Widget inputs are `CodeEditor`s carrying `editorTextFocus`** (§H.1), and a focused sticky line keeps text focus (§H.2), so host actions and text bindings run where Monaco's wouldn't. The bracket highlighter tests text focus where Monaco tests widget focus (§D.6). #472.
- **The LSP bridge keys documents by the raw uri** (§G.1), and the API passes the raw uri or `None` where Monaco passes the normalized `model.uri` (§G.6–§G.8). The bridge is #471, the API #490.
- **Brackets come from `colorizedBracketPairs` or `brackets`, not both** (§F.2), and the indentation processor strips only `brackets` (§F.4). #481.
- **The find seed and Cmd+D drop words with no letter or digit** (§A.6), through the same filter in `multi_cursor.rs` and `find.rs`. #485.

The decide-and-document items (smart select's syntax-tree ranges §D.7, the byte-range wording §G.10, the LSP feature lists §G.11) are tracked by a docs issue, #493. For each, copy Monaco or correct the docs. Findings whose fix sketch offers documenting as a fallback (§B.8, §C.25, §E.7, §F.5, §F.7, §H.9, §H.10) stay with their code issues.

No finding is blocked by GPUI.

## Counts

89 findings in the eight reports; 88 after dropping one that was wrong (F7 and the folding keys are already gated by their preconditions through `with_preconditions`, `actions.rs:2388-2389`) and moving one duplicated sub-item (the find seed's filter, from the widget report's find edge cases into §A.6): 4 P1, 23 P2 and 61 P3.

| Section | Findings | P1 | P2 | P3 |
|---|---|---|---|---|
| A. Editing and cursors | 9 | 1 | 3 | 5 |
| B. View rendering | 8 | 0 | 2 | 6 |
| C. Widgets and menus | 26 | 2 | 12 | 12 |
| D. Navigation, scrolling and folding | 7 | 0 | 1 | 6 |
| E. Diff editor and multi diff editor | 7 | 0 | 1 | 6 |
| F. Options, themes and languages | 10 | 0 | 2 | 8 |
| G. Public API and the LSP bridge | 11 | 1 | 1 | 9 |
| H. Input, accessibility and keybindings | 10 | 0 | 1 | 9 |
| **Total** | 88 | 4 | 23 | 61 |

## A. Editing and cursors

Scope: typing, IME, auto-closing, linked editing, multi-cursor and find-match selection, undo grouping, `cursorMove` and the `type`/`paste` commands.

Checked and matching: the audit 4 fixes for `nodeAcceptEdit` (`DecorationSet::apply_patch_forcing`, `decorations.rs:848-1072`, case by case against `common/model/intervalTree.ts:416-490`, with `collapse_on_replace_edit` and auto-closed ranges through `move_edge`), linked editing's default word pattern (`linked_editing.rs:212-223`) and reinitializing (option, provider registry, language and model changes), and outside undo restoring the focused editor's selections (`editor.rs:2992-2998`). Audit 4 §A.4 is incomplete (§A.1, §A.2).

### Input method edits (#469)

#### A.1 A single-cursor IME composition is one `Other` undo step per update, not `TypingOther` (P1)
Incomplete fix of audit 4 §A.4.
- **Monaco:** every EditContext text update goes through `_emitTypeEvent` → `_onType` (`browser/controller/editContext/native/nativeEditContext.ts:434-476`). The first update calls `type`; later ones replace the composition with `compositionType` (`common/cursor/cursor.ts:589-606`), a `TypingOther` edit with `shouldPushStackElementBefore: shouldPushStackElementBetween(prev, TypingOther)` and no stop after (`common/cursor/cursorTypeEditOperations.ts:800-804`). The updates, the commit, the typing before and the typing after all join one undo step.
- **Port:** with one cursor, `replace_and_mark_text_in_range` uses `composition_type_at_cursors` only when `multi` (`editor.rs:7104-7118`); otherwise it calls `replace_range`, and so does the commit in `replace_text_in_range` (the `(multi || marked.is_none())` guard, `editor.rs:7040-7053`). `replace_range` (`editor.rs:6928-6953`) is `kind: Other` with undo stops before and after, and `undo_group(Other)` is `None` (`editor.rs:666-675`), so every update is sealed as its own step. The commit also resets `prev_edit_kind` (`editor.rs:7054`). With several cursors the same input is `TypingOther` and joins. `MONACO_PARITY.md` says compositions join the typing's undo step.
- **Repro:** one cursor. Type `a`, then compose `nihon` with a Japanese IME (`mark("に")`, `mark("にほ")`, `mark("にほん")`) and commit `日本`. Undo: Monaco removes `a日本` in one step; the port turns `日本` back into `にほん` and takes one Undo per intermediate state to get back to `a`. Test sketch in `tests/ime.rs` style: `type_text("a")`, `mark("に")`, `mark("にほ")`, `commit("日本")`, `undo()`, assert the text is empty.
- **Fix:** run single-cursor updates and the commit through `composition_type_at_cursors` (or `cursor::composition_type`) as the multi-cursor path does, with the first update over a caret a plain `type`; stop using `replace_range` for composition text.

#### A.2 An IME or autocorrect replacement on the caret's line that doesn't contain the caret collapses the cursors and moves the caret (P2)
Incomplete fix of audit 4 §A.4.
- **Monaco:** `_emitTypeEvent` (`nativeEditContext.ts:434-469`) measures any update against the primary selection. A range before the caret gives `replacePrevCharCnt = selectionStart - updateRangeStart`, and the text between the range's end and the caret is appended to the inserted text (`:452-456`). It then becomes `compositionType` at every cursor (`:471-476`), so the caret stays after the replaced text plus that gap. Autocorrect replacing `teh` in `teh |` is `compositionType("the ", 4, 0, 0)`.
- **Port:** `composition_type_at_cursors` returns `None` unless `start <= head <= end` (`editor.rs:2877-2879`). Anything else falls through to `replace_range` (`editor.rs:7051-7054`), which edits once, collapses to the end of the new text and drops the other cursors (`editor.rs:6928-6953`). `replacing_text_away_from_the_cursors_replaces_it_once` (`crates/gpui-monaco/tests/ime.rs:173-195`) locks this in.
- **Repro:** one cursor after `teh ` (column 4); macOS autocorrect calls `replace_text_in_range(Some(0..3), "the")`. Monaco gives `the |` with the caret after the space; the port gives `the| `. With two cursors (the test above), Monaco edits both lines and keeps both cursors; the port edits one line and keeps one cursor.
- **Fix:** when the range is on the primary selection's line, build Monaco's `ITypeData` (replace-prev/next counts against the selection's edges, the gap text padded in, `positionDelta` from the IME selection) and run it through `composition_type` at every cursor; update the test.

### Commands and undo (#473)

#### A.3 `trigger` from a non-keyboard source still runs the typing interceptors and the type and paste side effects (P2)
- **Monaco:** `_type` fires `onWillType`/`onDidType` only for `source === 'keyboard'` (`browser/widget/codeEditor/codeEditorWidget.ts:1188-1199`). `Cursor.type` runs `typeWithInterceptors` (auto-close, auto-surround, Enter's auto-indent, electric characters) only from the keyboard, else `typeWithoutInterceptors` (`common/cursor/cursor.ts:564-587`). Suggest trigger characters, parameter hints and format-on-type listen to `onDidType` (`contrib/suggest/browser/suggestModel.ts:300`, `contrib/parameterHints/browser/parameterHintsModel.ts:90`, `contrib/format/browser/formatActions.ts:89`). `_paste` fires `onDidPaste` only from the keyboard (`codeEditorWidget.ts:1206-1222`), and format-on-paste and `AutoIndentOnPaste` listen to it (`formatActions.ts:198`, `contrib/indentation/browser/indentation.ts:400`).
- **Port:** `trigger_from` records the source in `trigger_source` (`api.rs:1472-1475`), but the `Type` handler always calls `type_text` (`editor_commands.rs:206`), which emits `WillType`/`DidType`, types through the interceptors and triggers suggest, signature help and format-on-type (`editor.rs:4037-4075`). `PasteText` always calls `paste_item_inner` (`editor_commands.rs:1098-1111`), which emits `DidPaste`, formats and auto-indents (`editor.rs:4612-4645`). Nothing reads `trigger_source` here, and `TypeWithoutInterceptorsOperation` isn't ported.
- **Repro:** TypeScript, auto-closing on. `editor.trigger_from("api", "type", Some(json!({"text": "("})))`: Monaco inserts `(`, the port `()`. `{"text": "\n"}` after `{`: Monaco inserts a bare newline, the port auto-indents. With a `.` trigger-character provider, the port opens suggest and Monaco doesn't.
- **Fix:** in the `Type` handler, when the source isn't the keyboard, apply Monaco's `TypeWithoutInterceptorsOperation` (a `ReplaceCommand` per selection, `getTypingOperation` kind) and skip the type events, suggest, signature help and format-on-type; in `PasteText`, skip `DidPaste`, format-on-paste and auto-indent-on-paste.

#### A.4 Linked editing's mirror edit after a paste, cut or other `Other` edit is its own undo step (P2)
- **Monaco:** `_syncRanges` calls `this._editor.popUndoStop()` before `executeEdits('linkedEditing', …)`, then restores the previous edit type (`contrib/linkedEditing/browser/linkedEditing.ts:242-247`). The mirror always joins the edit that triggered it, even one that pushed an undo stop after itself (paste's `PasteOperation`, `shouldPushStackElementAfter: true`).
- **Port:** `sync_linked_ranges` (`linked_editing.rs:255-266`) calls `apply_edits_moving_cursors` with `kind = prev_edit_kind` and no undo stops, but never reopens the last step (`TextModel::pop_stack_element`, `model.rs:1916`, isn't used here). After a paste (`cursor/typing.rs:1189-1260`, `Other`, stops before and after), the step is sealed and `undo_group(Other)` is `None`, so the mirror starts a new one. Typing and deleting happen to join; anything `Other` doesn't.
- **Repro:** HTML `<div></div>`, caret inside `div` with the linked ranges showing. Paste `abc`: both tags read `divabc`. Undo once: Monaco reverts both tags; the port reverts only the closing tag (`<divabc></div>`), and a second Undo reverts the paste.
- **Fix:** call `model.pop_stack_element()` before the mirror edit, as `popUndoStop` does, then push the stop back if the triggering edit had one.

#### A.5 `cursorMove` left, right and `halfLine` move by model columns, not view columns (P3)
- **Monaco:** `simpleMove` for `left`/`right` runs `MoveOperations.moveLeft/moveRight` on the cursor's view state (`common/cursor/cursorMoveCommands.ts:440-494`); `halfLine` uses half the view line's length (`viewModel.getLineLength(viewLineNumber)`, `:458`, `:482`).
- **Port:** `cursor_move` maps `left`/`right` to `cursor::move_left/move_right` on the model states, and `halfLine` to half of `rope.line_len_utf16(row)` (`editor_commands.rs:452-468`). The arrow keys themselves are view-based (`editor.rs:3622-3632`).
- **Repro:** word wrap on, a 200-column line wrapped at 80 columns, caret on the second segment. `cursorMove {to: "right", by: "halfLine"}`: Monaco moves 40 columns, the port 100. `cursorMove {to: "left"}` at the start of the line after a collapsed fold: Monaco lands at the end of the fold's header line; the port lands inside the hidden lines.
- **Fix:** run left, right and `halfLine` on `cursor_view_states()` through `ViewCursorModel`, as `move_cursors` does, and map back to the model.

### Find matches and multi-cursor (#485)

#### A.6 Cmd+D and the find seed refuse words with no letter or digit (P3)
Also found by the widgets audit.
- **Monaco:** `MultiCursorSession.create` and `_expandEmptyToWord` use `getConfiguredWordAtPosition` (`contrib/multicursor/browser/multicursor.ts:331`, `:571`), as does the find seed (`contrib/find/browser/findController.ts:52-56`). That is `WordOperations.getWordAtPosition` (`common/cursor/cursorWordOperations.ts:728-739`): any `WordType.Regular` run, meaning characters that aren't whitespace or in `wordSeparators`.
- **Port:** `word_selection` keeps the word only if `text.chars().any(|c| c.is_alphanumeric() || c == '_')` (`multi_cursor.rs:55-73`); `selection_search_string` does the same (`find.rs:1828-1836`).
- **Repro:** caret on `😀` in `a 😀 b 😀`, or on `→→`, or on `--` in `a -- b --` with `word_separators` minus `-`. Cmd+D: Monaco selects it and a second Cmd+D adds the next one; the port does nothing, and Cmd+F doesn't seed the search string.
- **Fix:** drop the alphanumeric filter and use `word_at`'s word only when it's a regular (non-separator) run, as `getWordAtPosition` does.

#### A.7 A Cmd+D session doesn't end when the editor text loses or gains focus (P3)
- **Monaco:** `_beginSessionIfNeeded` ends the session on `onDidBlurEditorText`, and on `onDidFocusEditorText` unless `selectedTextMatchMode` is `findOptions` (`multicursor.ts:527-533`). `_endSession` clears the word session's whole-word, match-case and regex overrides on the find state (`:545-555`).
- **Port:** the session ends only when the selections, the find toggles or the match mode option change (`multi_cursor.rs:122-151`, `editor.rs:1577-1579`). The blur and focus handlers (`editor.rs:995-1021`) don't touch `self.multi_cursor`.
- **Repro:** caret on `foo`, Cmd+D (whole word and match case forced on), then Cmd+F. Monaco drops the overrides, so the toggles show their own values; the port keeps them forced on, and back in the editor the next Cmd+D still searches whole-word and case-sensitively.
- **Fix:** call `end_multi_cursor_session` in the text blur handler, and in the focus handler unless the mode is `FindOptions`.

#### A.8 Move Selection to Next Match and Add Selection to Previous Match expand every empty cursor to its word (P3)
- **Monaco:** only `addSelectionToNextFindMatch` expands all selections to words, and only without a session when the selections don't share their text (`multicursor.ts:596-614`). The other three (`:622-641`) begin a session whose `currentMatch` is the primary's word (`:328-336`); the first step returns it: add appends it, and move replaces the *last* selection with it (`:376-391`).
- **Port:** `session_step`, shared by all four, starts with `expand_empty_selections`, which expands every empty selection (`multi_cursor.rs:239-272`, `:332-337`).
- **Repro:** two empty cursors, the primary in `foo` on line 1 and another in `bar` on line 2. Cmd+K Cmd+D: Monaco replaces the `bar` cursor with the `foo` selection; the port selects both `foo` and `bar`.
- **Fix:** expand all selections only for add-next; for the others, begin the session and use the primary's word as the first result.

#### A.9 Select All Occurrences stops at 19,999 matches (P3)
- **Monaco:** `MultiCursorSession.selectAll` and the regex path pass `Constants.MAX_SAFE_SMALL_INTEGER` to `findMatches` (`multicursor.ts:466-468`, `:661-663`); only `multiCursorLimit` (up to 100,000) caps the cursors.
- **Port:** `occurrences_in_scope` passes `crate::find::MATCHES_LIMIT` (19,999, `find.rs:26`) to `search::find_matches` (`multi_cursor.rs:468-486`).
- **Repro:** `multi_cursor_limit: 50_000`, a model with 30,000 `x`s, Cmd+Shift+L on one: Monaco makes 30,000 cursors, the port 19,999.
- **Fix:** pass `usize::MAX` (or `multi_cursor_limit`) instead of the find widget's limit.

## B. View rendering

Scope: view lines, wrapping, whitespace and control characters, cursors, selections, the decorations overlay, guides, rulers, the gutter, the minimap, the overview ruler, scrollbars, sticky scroll, view zones, injected text and unicode highlighting.

Checked and matching: every audit 4 §B fix: wrap-point affinity (`element.rs:7706-7745`), merged touching decorations (`element.rs:1478-1554`), selection highlights fading when unfocused (Monaco's `.focused` is toggled on the overlay node, `browser/view/viewOverlays.ts:125`), the 2px current match and dashed high-contrast borders, the blink reset in `update_options`, the grapheme-wide block cursor, bracket pairs at an off-screen cursor, the DOM continuation indent, whitespace over the view line, NUL as U+FFFD, the half-width from `n`, the minimap fold band and inline mark, gutter parts on the view range, 16px glyph codicons and the kept `lineNumbers: interval` quirk.

### Decorations and the gutter (#483)

#### B.1 Decoration glyphs are drawn when the glyph margin is off (P2)
- **Monaco:** with `glyphMargin: false`, `prepareRender` empties `_decorationGlyphsToRender` and `render` removes every glyph node (`browser/viewParts/glyphMargin/glyphMargin.ts:338-341`, `:391-399`).
- **Port:** `element.rs:5048-5058` queues a `GlyphRequest` for every decoration with a `glyph` without checking the margin; only widget requests are inside `if glyph_width > px(0.)` (`element.rs:5995-6018`). The draw loop places each glyph at `left + (lane_width - shaped.width) / 2` (`element.rs:6021-6049`); with the margin off `lane_width` is 0, so the glyph is centered on the gutter's left edge, half clipped and half over the line numbers. `glyph_margin` defaults to `false` (`options.rs:2267`, standalone Monaco's default).
- **Repro:** default options and a decoration with `glyph: Some(("●".into(), red))` on line 3. Monaco shows nothing; the port draws the right half of a dot at the editor's left edge.
- **Fix:** skip decoration glyph requests when `layout.glyph_margin_width` is zero.

#### B.2 The gutter's line, glyph and margin parts use the `showIfCollapsed`-shortened end (P3)
- **Monaco:** only `DecorationsOverlay._renderNormalDecorations` pulls a `showIfCollapsed` range ending at column 1 back to the previous line's end (`browser/viewParts/decorations/decorations.ts:158-161`). `LinesDecorationsOverlay`, `MarginViewLineDecorationsOverlay` and `GlyphMarginWidgets` use the decoration's own view range (`browser/viewParts/linesDecorations/linesDecorations.ts:72-88`).
- **Port:** the per-decoration loop derives `view_start..view_end` from `collapsed_end(..)` (`element.rs:4834-4846`), and the gutter parts use that range: bars, line decoration text, `margin_background`, `line_number_color` and glyphs (`element.rs:4983-5058`).
- **Repro:** a non-whole-line decoration with `show_if_collapsed: true`, `glyph` and `margin_background` over `(2,0)..(4,0)`. Monaco marks lines 3–5 (1-based); the port lines 3–4.
- **Fix:** compute the gutter view range from the original end, keeping `collapsed_end` for the overlay only.

#### B.3 Indent and bracket guides are placed by the typical half-width, not the space width (P3)
Exposed by audit 4 §B.11 measuring `em_width` from `n`.
- **Monaco:** `IndentGuidesOverlay` puts a guide at `leftOffset + (visibleColumn - 1) * this._spaceWidth`, `_spaceWidth = fontInfo.spaceWidth` (`browser/viewParts/indentGuides/indentGuides.ts:43`, `:134-151`); only `_maxIndentLeft` uses `typicalHalfwidthCharacterWidth` (`:44`).
- **Port:** `metrics.em_width` places indent guides (`element.rs:5641`) and column-less bracket guides (`element.rs:5596`), while the indentation itself is laid out with `space_width`.
- **Repro:** `font_family: "Helvetica"`, 4-space indents, `guides.indentation: true`. Monaco's level-2 guide sits on the first character of the indented code; the port's sits about twice as far right, inside the code. Monospace fonts are unaffected.
- **Fix:** use `metrics.space_width` for the guide x in both places, keeping `em_width` for the max indent.

#### B.4 Line numbers don't use tabular figures (P3)
- **Monaco:** `.margin-view-overlays .line-numbers { font-variant-numeric: tabular-nums; }` (`browser/viewParts/lineNumbers/lineNumbers.css:8`).
- **Port:** numbers are shaped in `metrics.font` without a `tnum` feature (`element.rs:5862-5867`, the font from `element.rs:250-256`); sticky line numbers likewise (`element.rs:4628-4635`).
- **Repro:** `font_family: "Helvetica"`: lines 11 and 18 have different widths in the port, so their left edges don't line up; in Monaco they do.
- **Fix:** shape line numbers with a copy of the font whose features add `("tnum", 1)`.

### Token colors outside the main view (#484)

#### B.5 Sticky scroll lines and the minimap leave out semantic token colors (P2)
- **Monaco:** model line tokens include semantic tokens (`common/model/tokens/tokenizationTextModelPart.ts:163`, `addSparseTokens`). Sticky lines render `getViewLineRenderingData(..).tokens` (`contrib/stickyScroll/browser/stickyScrollWidget.ts:452`, `:469`) and the minimap colors from `getMinimapLinesRenderingData` (`browser/viewParts/minimap/minimap.ts:1973`, `:2001`), so both show semantic colors.
- **Port:** only the main view merges them (`merge_semantic_tokens`, `element.rs:3871`). Sticky lines use `current_highlights` alone (`element.rs:4569-4580`); minimap tiles use `HighlightSource::new`, syntax only (`minimap.rs:511`, `render_lines` at `:1060-1100`).
- **Repro:** a semantic token provider that colors a parameter differently from its tree-sitter capture, sticky scroll on. The stuck function header shows the syntax colors the text line didn't, and the minimap never shows the semantic colors.
- **Fix:** run `merge_semantic_tokens` on the sticky rows' highlights; snapshot the semantic store into the minimap tile build and merge per line.

#### B.6 The minimap leaves out injected text (P3)
- **Monaco:** minimap lines are view line data (`common/viewModel/viewModelImpl.ts:933-939` → `getViewLinesData`), whose content includes injected text (`common/viewModel/modelLineProjection.ts:159-191`), so inlay hints, `before`/`after` content and ghost text show and shift the rest of the line.
- **Port:** `render_lines` builds each minimap line from the segment's model text plus the wrap indent (`minimap.rs:1110-1144`), though the view lines and wrapping count injected text.
- **Repro:** TypeScript parameter-name inlay hints, minimap on: Monaco's minimap shows the hints and shifts the line; the port's shows the bare line.
- **Fix:** build minimap lines from the view line content (`view_lines.line_content(view_line)`, as whitespace does at `element.rs:5701`), injected text in the default or its own token color.

#### B.7 `editor.selectionForeground` recolors selected text in every theme, not only high contrast (P3)
- **Monaco:** the `inline-selected-text` decoration is added only when `isHighContrast(options.themeType)` (`browser/viewParts/viewLines/viewLine.ts:124-140`), and the color rule targets that class (`browser/viewParts/selections/selections.ts:400-405`).
- **Port:** `element.rs:4010-4040` overlays `colors.selection_foreground` whenever the theme sets it (`theme.rs:120`), with no high-contrast check.
- **Repro:** `define_theme` on `vs-dark` with `"editor.selectionForeground": "#ff0000"`, select a word: Monaco keeps the token colors, the port paints it red.
- **Fix:** apply the overlay only when `theme.high_contrast`.

### Font options (#486)

#### B.8 There's no `allowVariableFonts`, so font decorations aren't dropped in accessibility mode (P3)
- **Monaco:** `effectiveAllowVariableFonts` is `allowVariableFontsInAccessibilityMode` (default `false`) with a screen reader, else `allowVariableFonts` (default `true`) (`common/config/editorOptions.ts:2094-2110`, `:6064-6075`). When false, `filterFontDecorations` drops every decoration with `fontFamily`, `fontSize`, `fontWeight` or `fontStyle` from the view, its background, gutter and overview parts included (`common/config/editorOptions.ts:4001-4003`, `common/viewModel/viewModelDecorations.ts:32`, `common/model/textModel.ts:2545`, `common/model/intervalTree.ts:113-118`).
- **Port:** neither option exists in `EditorOptions`. The render path filters decorations only by `validation` (`element.rs:3872-3880`, `:4544-4552`); `has_font()` and `filter_font_decorations` serve only the getter API (`api.rs:1821-1836`).
- **Repro:** `accessibility_support: On` and a decoration with `font_weight: Some(BOLD)` and a background: Monaco shows neither, the port both.
- **Fix:** add both options, compute the effective value from `accessibility_support`, and leave `has_font()` decorations out of the view queries when it's false; or list the difference in `book/src/differences.md`.

## C. Widgets and menus

Scope: suggest, parameter hints, content and glyph hovers, markdown, find, quick input, rename, code actions and the lightbulb, code lenses, peek, the marker zone, color pickers, the context menu, the inline completions toolbar and inline edits, paste and drop widgets, inline progress and the message widget.

Checked and matching: the audit 4 §C fixes, spot-checked: `editorTextFocus` gating of focused widgets (`actions.rs:2445-2483`), focusable hover buttons, the find widget's `FocusNextElement`, "Loading...", no hover under the context menu, half-alpha separators, task lists and `kbd`, `force_above`, the suggest context keys, keys while a message shows, "Insert" without distinct ranges, high-contrast borders, read-more, the parameter hints scrollbar, Enter through `find_match_action`, the find slide-out, the Close tooltip, `IndexZero`, tracked color ranges, `swatch_at`, the selection-start lightbulb, code lens reconciliation, peek mode and toggling, the last-line reveal, markers in files and the docked tab icons. Audit 4 §C.22 and §C.35 are incomplete (§C.25, §C.14).

### Suggest word ranges and accepting (#470)

#### C.1 Suggest word boundaries ignore the language's `wordPattern` (P1)
- **Monaco:** the default insert and replace ranges come from `model.getWordAtPosition` (`contrib/suggest/browser/suggest.ts:226`), the language's word definition; so do `shouldAutoTrigger` (`suggestModel.ts:75`), `LineContext.leadingWord` (`getWordUntilPosition`, `:97`) and `atEndOfWord` (`contrib/suggest/browser/wordContextKey.ts:54`). The default regex treats `$` and `-` as separators; CSS, SCSS and LESS include `-` in words.
- **Port:** `is_word_char` (alphanumeric, `_`, `$`; `completion.rs:432-434`) drives `word_start_at`/`word_end_at` (`completion.rs:689-722`), which feed the default ranges, `should_auto_trigger`, the quick-suggest gate and the word-based items' ranges, while the words themselves are collected with the language regex (`completion.rs:971-999`). `at_end_of_word` (`context_keys.rs:589-605`), `suggest_inline_completions.rs:148-185` and `insert_best_completion` (`completion.rs:1101-1108`) hard-code it too. `TextModel::word_at_position` (`api.rs:231`) uses the language pattern but none of these call it.
- **Repro:** CSS with `font-weight` in the file. On a new line type `font-w` and accept the word-based `font-weight`: the port replaces only `w` and gives `font-font-weight`; Monaco gives `font-weight`. Typing `-` also ends the port's word, which stops quick suggestions.
- **Fix:** base `word_start_at`/`word_end_at`, `should_auto_trigger`, `at_end_of_word` and the inline `word_at` on the language word pattern (`word_at_position`/`word_until_position`).

#### C.2 Accepting after moving the caret right inside the word deletes text after it (no `LineSuffix` delta) (P1)
- **Monaco:** `suggestController.ts:235` puts a `LineSuffix` marker at the trigger position (`:54-104`). `getOverwriteInfo` (`:573-588`) adds `suffixDelta`, the marker offset minus the cursor offset, to `overwriteAfter`; after the caret moves right with no typing it is negative.
- **Port:** `overwrite_after = end - session.position.column` (`completion.rs:2533-2534`), with no suffix correction, and `insert_snippet` applies it from the current head (`snippet_session.rs:395-404`). Right and Left keep the session open and refilter (`editor.rs:3780-3786`).
- **Repro:** `suggest.insert_mode = Replace`, line `foobar = 1`. Ctrl+Space after `fo`, Right twice (`foob|ar`), Enter on `foobar`: the port gives `foobar 1` (it deletes `ar =`); Monaco gives `foobar = 1`.
- **Fix:** subtract `head.column - session.position.column` from `overwrite_after` (saturating), or track a `LineSuffix`-style anchor.

#### C.3 Commit characters accept the focused item while the list is frozen (P2)
- **Monaco:** `contrib/suggest/browser/suggestCommitCharacters.ts:39` accepts only when `!widget.isFrozen()`.
- **Port:** `commit_suggestion_on` (`completion.rs:2653-2671`) checks only `session.focused`. A manual session that stops matching sets `frozen` but keeps `focused` (`completion.rs:1974-1991`). The check runs on every typed character (`editor.rs:4058-4059`).
- **Repro:** an item `foo` with commit characters `["."]`. Type `fo`, Ctrl+Space, type `x` (the list freezes on `foo`), then `.`: the port inserts `foo.`, Monaco leaves `fox.`.
- **Fix:** return false when `session.frozen`.

### Suggest and parameter hints (#475)

#### C.4 Items without `sortText` aren't sorted by label against items that have one (P2)
- **Monaco:** a missing `sortText` is filled with the label (`contrib/suggest/browser/suggest.ts:250-253`) before `defaultComparator` compares `sortTextLow` (`:339-356`).
- **Port:** `suggest/completion_model.rs:375-378` keeps `None`, and `provider_compare` (`:384-399`) compares sort text only when both items have one, else falls through to the label. `CompletionItem::sort_text()` (`providers.rs:532`) has the label fallback but isn't used here.
- **Repro:** a provider returns `{label: "aaa", sortText: "zzz"}` and `{label: "bbb"}`; Ctrl+Space with no prefix. Monaco lists `bbb` first, the port `aaa`.
- **Fix:** build `sort_text_low` from `item.sort_text()`.

#### C.5 Additional edits that arrive with the resolve are a separate undo step (P2)
- **Monaco:** an unresolved item is inserted with `NoAfterUndoStop` (`contrib/suggest/browser/suggestController.ts:396-397`); the resolved additional edits join the insertion and the undo stop comes after them, unless the user typed in between (`:399-425`).
- **Port:** `insert_completion` always ends with a stack element (`completion.rs:2566`), and `apply_late_edits` pushes stack elements on both sides of its edits (`completion.rs:2500-2504`).
- **Repro:** an item whose `resolve_completion_item` adds an import on line 0. Accept it, wait for the resolve, Undo: Monaco removes the import and the insertion together; the port only the import.
- **Fix:** skip the closing stack element while a resolve is pending; in `apply_late_edits`, push one before only if the user typed since.

#### C.6 Additional edits on the cursor line aren't shifted by the text typed since the request (P3)
- **Monaco:** an additional edit on `item.position`'s line that starts after `item.position.column` is shifted by `cursor.column - item.position.column` (`suggestController.ts:364-372`).
- **Port:** `insert_completion` applies `item.additional_text_edits` with the provider's ranges unchanged (`completion.rs:2539-2541`, `:2570-2576`).
- **Repro:** at `x(|)`, an item with an additional edit inserting `;` at column 3. Type `ab`, accept: Monaco puts `;` after `)`, the port two columns early.
- **Fix:** shift such edits by `head.column - session.position.column`.

#### C.7 An open suggest session ignores language and completion provider changes (P3)
- **Monaco:** the model cancels on `onDidChangeModelLanguage` (`contrib/suggest/browser/suggestModel.ts:174-177`), re-requests an open session when completion providers change and cancels it when none remain (`:181-184`, `:335-343`).
- **Port:** the `LanguageChanged` handler (`editor.rs:3069-3092`) doesn't call `hide_suggest`, and registering or removing a completion provider (`providers.rs:3084`, `:3345`) doesn't reach open sessions.
- **Repro:** Ctrl+Space, then `model.set_language("css")` or register a second provider: the port keeps the old list.
- **Fix:** `hide_suggest` on `LanguageChanged`; a provider epoch observer that re-requests or hides.

#### C.8 Signature help asks every provider at once and skips a first provider's empty answer (P3)
- **Monaco:** `contrib/parameterHints/browser/provideSignatureHelp.ts:32-44` asks providers in order and returns the first non-null result, even one with no signatures; `parameterHintsModel.ts:216-220` then hides.
- **Port:** `request_signature_help` starts every provider up front (`signature_help.rs:509-514`) and takes the first non-empty result (`:518-523`).
- **Repro:** the higher-ranked provider returns `{signatures: []}`, a lower one a signature: Monaco shows nothing and never asks the second; the port shows the second's.
- **Fix:** await providers in order and stop at the first `Some`.

### Content hover (#474)

#### C.9 The hover is anchored at the nearest caret position, not the character under the mouse (P2)
- **Monaco:** for content text, the mouse target's `range` is the character under the mouse (`browser/controller/mouseTarget.ts:925-944`), and that is the hover anchor (`contrib/hover/browser/contentHoverWidgetWrapper.ts:279-281`). A marker or decoration counts only if it contains that character (`contentHoverComputer.ts:43-62`); providers are asked at its start (`markdownHoverParticipant.ts:183`).
- **Port:** the position comes from `closest_index_for_x`, which rounds to the nearer edge (`editor.rs:6087-6101`, `element.rs:161-172`). Markers match on `start <= offset <= end` (`markers.rs:547-560`), decorations are excluded only by `start > column || column > end` (`hover.rs:2945-2949`), and providers get the rounded position.
- **Repro:** `let x = foo;` with an error on `foo`: resting on the left half of `;` shows the error in the port, nothing in Monaco. In `foo.bar`, the right half of `.` asks the provider about `bar` in the port.
- **Fix:** anchor on the character (floor column to the next), ask providers at its start, and filter markers and decorations by containment.

#### C.10 A hover isn't kept while the mouse moves toward it (`isMouseGettingCloser`) (P2)
- **Monaco:** after the hiding-delay grace (`contrib/hover/browser/contentHoverController.ts:239-243`), the card stays while the mouse's distance to it hasn't grown by more than 4px (`contentHoverWidget.ts:238-276`, `contentHoverWidgetWrapper.ts:115-123`). An empty recomputation keeps the old card (`insistOnKeepingHoverVisible`, `contentHoverWidgetWrapper.ts:151-162`, `:206-214`).
- **Port:** `hover_mouse_move` runs the grace timer once (`hover.rs:2317-2334`); `react_to_hover_mouse` then hides the card unless the mouse is in its range or on its line (`hover.rs:2388-2418`). There's no distance tracking.
- **Repro:** `hover.above: true`; hover a word on line 10 and move slowly up and right through blank space toward the card. Monaco keeps it; the port hides it about 300ms after leaving the word.
- **Fix:** record the card bounds and the closest distance, and keep the card while the mouse isn't more than 4px farther away.

#### C.11 The card doesn't hide when the mouse leaves it to outside the editor (P2)
- **Monaco:** `mouseleave` on the content hover hides it when the mouse is outside the editor (`contrib/hover/browser/contentHoverWidgetWrapper.ts:83-85`, `:309-315`); the glyph hover likewise (`glyphHoverWidget.ts:57-58`, `:203-208`).
- **Port:** the hover card has no hover or leave handler (`hover.rs`). Editor mouse moves are forwarded only over the editor's hitbox or the overflow bounds (`element.rs:3255-3266`), and the editor's `on_hover(false)` (`editor.rs:8243-8257`) fired when the mouse went onto the card.
- **Repro:** a card that overflows the editor's edge. Move onto the overflowing part, then off into the rest of the window: Monaco hides the card; the port leaves it until the next editor event.
- **Fix:** an `on_hover(false)` on the hover roots that hides the card when the mouse isn't over the editor.

#### C.12 A focused hover loses focus when streamed parts rebuild it, and hiding it doesn't refocus the editor (P2)
Related to audit 4 §C.1 and §C.4.
- **Monaco:** a new result swaps only the contents node, so the focused container keeps focus (`contrib/hover/browser/contentHoverWidget.ts:295-320`, `:348-350`); `hide()` returns focus to the editor if the hover had it (`:368-376`).
- **Port:** each result goes through `show_hover_card`, which removes the widget and creates a new `HoverWidget` with a new focus handle and `focused_part: None` (`hover.rs:3048-3050`, `:3111-3136`). `hide_hover_card` (`hover.rs:1997-2005`) never sets `refocus`; only View Problem does (`hover.rs:1648`).
- **Repro:** a marker plus a hover provider that answers after 1s. Cmd/Ctrl+K Cmd/Ctrl+I twice focuses the marker card; when the provider answers, nothing has focus and typing goes nowhere. Focusing the hover and then wheel-scrolling the text does the same.
- **Fix:** carry focus (root or matching row) over to the rebuilt widget, and refocus the editor when a hidden card had focus.

#### C.13 Hover verbosity: the command without a focused part, and the +/- buttons while scrolling (P3)
- **Monaco:** with no focused part, Increase/Decrease Hover Verbosity updates every markdown part that supports it (`contrib/hover/browser/contentHoverRendered.ts:407-421`, `:485-491`). `handleScroll` keeps a part's verbosity actions at the bottom of its visible area (`markdownHoverParticipant.ts:352-372`, wired at `contentHoverWidgetWrapper.ts:63-65`).
- **Port:** `update_markdown_hover_verbosity` picks only the first supporting part (`hover.rs:1863-1869`); the buttons are `justify_end` at the bottom of the whole part regardless of scroll (`hover.rs:1184-1194`).
- **Repro:** two providers with `canIncreaseVerbosity`; run the command from the palette: the port expands only the first. With a part taller than the card, the port's +/- show only after scrolling to its end.
- **Fix:** loop over all supporting parts when no part is focused; offset the buttons by the scroll position.

### Code lens and code actions (#477)

#### C.14 Code lens zones don't follow their line during edits (P2)
Incomplete fix of audit 4 §C.35.
- **Monaco:** on every content change, `codelensController.ts:200-233` re-anchors each lens to its tracked range (`codelensWidget.ts:339-350`) and drops lenses whose range is gone or now shares a line (`codelensController.ts:207`, `codelensWidget.ts:271-276`).
- **Port:** a zone is anchored at the end of the line above (`code_lens.rs:266-277`, `editor.rs:2053-2069`) and isn't touched until the next request returns, at least 250ms later (`code_lens.rs:336-424`). Matching is by row (`code_lens.rs:454-478`), so a drifted zone is removed and rebuilt with only the resolved lenses: it blanks, then fades in. `Group::row` goes stale for `resolve_visible_code_lenses` and `show_lenses_in_current_line`.
- **Repro:** a lazily resolved lens on `fn foo()` (row 5). Enter at the end of row 4: the port's lens sits over the new blank line, blanks, then reappears over `fn foo()`. Delete row 4 with Cmd+Shift+K and the lens briefly draws below `fn foo()`.
- **Fix:** on `Edited`, re-anchor zones from a tracked range of each group's first lens and drop collapsed or duplicate rows.

#### C.15 Code lens refresh and resolve timing: no refresh on focus, no resolve debounce or cancellation (P3)
- **Monaco:** gaining text focus schedules a request and blur cancels one (`contrib/codelens/browser/codelensController.ts:235-240`). Resolves run from a `RunOnceScheduler` with a moving-average delay of at least 250ms after scroll and layout (`:57`, `:241-248`), each run cancels the last (`:381`), and edits cancel in-flight resolves (`:229-232`).
- **Port:** requests happen only on buffer version or provider epoch changes (`code_lens.rs:347-372`). `resolve_visible_code_lenses` runs every frame without delay (`element.rs:3907`) and isn't cancelled (`code_lens.rs:597-598`).
- **Repro:** a provider whose "N references" grows per call: click out and back in, and Monaco's count updates while the port's doesn't. Fling-scroll a file with a lens per function: the port resolves every lens that passes through the viewport.
- **Fix:** bump a refresh epoch on focus; route resolves through a delayed, replaceable task that edits drop.

#### C.16 The lightbulb isn't hidden when a drag-select passes over it (P3)
- **Monaco:** `contrib/codeAction/browser/lightBulbWidget.ts:217-224` hides the bulb on `mouseenter` while the primary button is down.
- **Port:** the bulb is cleared only by request results, option or provider changes or a model swap (`code_action.rs:1030-1047`, `:1192`); the mouse handling only hit-tests it on mouse down (`editor.rs:5614-5631`).
- **Repro:** bulb on row 10, drag-select from row 9 down across it: Monaco's bulb vanishes, the port's stays.
- **Fix:** in mouse move with the left button pressed, clear the bulb when the pointer enters `layout.lightbulb`.

#### C.17 Clicking a lightbulb whose only fix is an AI fix re-requests code actions afterwards (P3)
- **Monaco:** `contrib/codeAction/browser/codeActionController.ts:146-158` runs the lone AI fix with `retrigger: false`.
- **Port:** `show_lightbulb_menu` calls `apply_code_action(item, true, false, cx)` (`code_action.rs:1354`), so `schedule_auto_code_actions` runs after it (`:1747-1749`).
- **Repro:** one valid action with `is_ai: true` and a command; click the bulb. The port makes an extra Auto `provide_code_actions` call 250ms later.
- **Fix:** pass `retrigger: false` on that path.

### Inline toolbar, color pickers, paste/drop and the message (#476)

#### C.18 The always-shown inline toolbar loses "1/N" and Previous/Next after the first keystroke (P2)
- **Monaco:** while the toolbar has a position, an autorun calls `model.triggerExplicitly()` whenever the last trigger wasn't explicit (`contrib/inlineCompletions/browser/hintsWidget/inlineCompletionsHintsWidget.ts:94-103`), so typing keeps the count. Clearing the count and disabling the buttons are debounced by 100ms (`:215-220`, `:262-280`).
- **Port:** `refresh_inline_toolbar` returns early when the position is unchanged and triggers explicitly only when `previous.is_none()` (`inline_completions/controller.rs:2699-2711`). Typing fetches with `Automatic` and clears `explicit`, so `inline_suggestion_index` returns `None` (`controller.rs:2916-2919`), and the toolbar blanks the count and disables the arrows at once (`inline_completions/toolbar.rs:233-259`).
- **Repro:** `show_toolbar = Always`, a provider with three items, "1/3" showing. Type one matching character: Monaco keeps the count and enabled arrows; the port shows an empty count and greyed arrows.
- **Fix:** re-trigger explicitly whenever the toolbar shows and the last trigger wasn't explicit; keep the last count and enabled state for 100ms.

#### C.19 The color pickers stay open after edits they didn't make, then write at stale coordinates (P2)
Related to audit 4 §C.30.
- **Monaco:** any content change the picker didn't make hides the hover picker and refocuses the editor (`contrib/colorPicker/browser/hoverColorPicker/hoverColorPickerParticipant.ts:135-142`); the standalone picker does the same (`standaloneColorPicker/standaloneColorPickerParticipant.ts:62-70`).
- **Port:** the hover hides only for edits through `apply_inner` (`editor.rs:3177-3180`). `on_model_event` (`editor.rs:2906` onward), which carries edits from another editor on the model, host edits and `set_value`, hides neither picker. The standalone picker closes only on cursor changes (`color_picker.rs:984-991`), and the picker's range moves only with its own edits (`color_picker.rs:268-279`).
- **Repro:** two editors on one model with `color: #ff0000;`. Open the hover picker in A, type `xx` at the start of the line in B, pick a color in A: Monaco had closed the picker; the port overwrites the wrong seven characters.
- **Fix:** on `Edited` while the picker isn't editing, hide a hover that holds a picker and the standalone picker, and refocus.

#### C.20 A failing paste/drop edit resolve still applies the unresolved edit (P2)
- **Monaco:** a failed `resolve()` reports "Error resolving edit" and applies nothing; with `canShowWidget` it shows the options widget instead (`contrib/dropOrPasteInto/browser/postEditWidget.ts:204-221`).
- **Port:** `resolve_post_edit` logs the error and still calls `apply(this, edit, cx)` with the unresolved edit (`drop_paste.rs:948-958`).
- **Repro:** two paste providers, the first's `resolve_document_paste_edit` returns `Err`. Paste: Monaco inserts nothing and shows the options button; the port inserts the unresolved edit.
- **Fix:** skip `apply` on `Err` and show the post-edit button for the remaining edits.

#### C.21 Re-picking the active paste/drop option removes the options button (P3)
- **Monaco:** picking the already active edit only hides the list; the `PostEditWidget` stays (`contrib/dropOrPasteInto/browser/postEditWidget.ts:146-150`). (The quick pick in place of the action list is documented, `book/src/differences.md:39`.)
- **Port:** the pick callback takes `post_edit` and removes its widget before checking `ix == post_edit.active` (`drop_paste.rs:1143-1150`).
- **Repro:** paste with two providers, open the options and pick the checked item: the port's button disappears.
- **Fix:** return before removing the widget when `ix == active`, putting `post_edit` back.

#### C.22 The inline message's mouse-move close bounds are one row too generous (P3)
- **Monaco:** `Range(line - 3, 1, mouseLine + 3, 1)` with `containsPosition` (`contrib/message/browser/messageController.ts:109-115`), so on the last row only column 1 is inside.
- **Port:** the rows `message_row - 3..row + 4` (`message.rs:374`), so the whole of row `mouseRow + 3` is inside.
- **Repro:** "No definition found" on line 10, mouse onto line 10, then to column 5 of line 13: Monaco closes the message, the port keeps it.
- **Fix:** treat the last row as inside only at column 0.

### Quick access (#478)

#### C.23 Go to Offset counts CRLF line breaks as one character (P2)
- **Monaco:** `contrib/quickAccess/browser/gotoLineQuickAccess.ts:167` uses `model.getValueLength()` and `:188` `model.getPositionAt(offset)`; both count CRLF as two.
- **Port:** `goto_offset_item` uses `rope.len_utf16()` and `rope.offset_utf16_to_offset` on the LF-only rope (`quick_access.rs:1404-1433`). `TextModel::value_length`/`get_position_at` (`api.rs:342-372`) count Monaco's way but aren't used.
- **Repro:** CRLF model `ab\r\ncd`, `::`. Monaco says "from 1 to 6" and `::5` goes to `c`; the port says "from 1 to 5" and goes to `d`. The error grows by one per line.
- **Fix:** use `value_length()` and `get_position_at()`.

#### C.24 Go to Symbol: enclosing-symbol activation repeats on every empty query; no "no symbol information" or busy state (P3)
Related to audit 4 §C.27.
- **Monaco:** only the first population gets `positionToEnclose`, and only with an empty query (`contrib/quickAccess/browser/gotoSymbolQuickAccess.ts:193-201`, `:221-222`). Names are trimmed (`:275`). Without a symbol provider it says "The active text editor does not provide symbol information." (`:81-95`) and waits for one (`:120-128`). `picker.busy` is set while symbols load (`:185`, `:191`, `:217`).
- **Port:** `quick_access.rs:1587-1593` activates and previews the enclosing symbol on every refresh whose `:`-stripped query is empty, so `@:` counts too. The name isn't trimmed (`:1517`). Without providers `outline.rs:113-125` returns an empty list and the box says "No editor symbols" (`quick_access.rs:1540-1546`). There's no busy state while `fetch_quick_symbols` runs (`:1456-1460`).
- **Repro:** cursor inside `fn b`; Cmd/Ctrl+Shift+O, type `x`, Backspace: the port re-activates and previews `b`, Monaco keeps the first item. In plain text the port says "No editor symbols". A provider that answers after 1s shows an empty box with no progress bar.
- **Fix:** pass the enclosing position only on the first population; tell "no provider" apart from "empty"; add a busy progress bar.

### Find widget (#485)

#### C.25 Toggling find options with find open doesn't flash the find input's toggles (P3)
Incomplete fix of audit 4 §C.22.
- **Monaco:** `contrib/find/browser/findController.ts:524-533` calls `_widget.highlightFindOptions()` when find is revealed (`findWidget.ts:852-854`); multicursor sessions reach the same path.
- **Port:** `highlight_find_options` returns early when `find.visible` (`find.rs:788-798`); its comment says this isn't ported, but neither `MONACO_PARITY.md` nor `differences.md` lists it.
- **Repro:** with find open, Cmd/Ctrl+D on a word or toggle Match Case from the palette: Monaco flashes the toggles, the port does nothing.
- **Fix:** add a brief highlight state to `FindWidget`, or document the gap.

#### C.26 Find widget edge cases: narrow thresholds and the seed length limit (P3)
- **Monaco:** `contrib/find/browser/findWidget.ts:486` grows `MAX_MATCHES_COUNT_WIDTH`, which the narrow and collapsed thresholds use (`:776-783`). `findController.ts:58` measures the seed limit in UTF-16 units.
- **Port:** `find_widget.rs:1349` grows `matches_width`, but the thresholds keep the constant 69px (`find_widget.rs:260-264`). `find.rs:1841` measures the limit in bytes. (The seed's alphanumeric filter is §A.6.)
- **Repro:** "1 of 19999+" switches to narrow at a different width than Monaco. A selection of 200k CJK characters doesn't seed in the port.
- **Fix:** thresholds from `self.matches_width`; compare the UTF-16 length.

## D. Navigation, scrolling and folding

Scope: reveal and scrolling, go to definition and references, bracket matching, word highlights, smart select, folding commands and mouse handling, and key preconditions.

Checked and matching: every audit 4 §D fix: `selection_range()` for the at-cursor check, symbol navigation and `open_location` (`goto.rs:429-432`, `:536-537`, `:630`), the alternative command's own config, centered immediate reveals, the peek's `Goto`/`Open` selecting the whole range, minimal reveals without horizontal padding, one-line autoscroll jumps, no inertia for line deltas, `revealLine`'s `parse_int`, the 999 textual word highlight cap and Remove Brackets' one `apply` per pair.

### Folding (#479)

#### D.1 Folding commands do nothing before the first range computation (P2)
- **Monaco:** `FoldingAction.runEditorCommand` awaits `foldingController.getFoldingModel()` (`contrib/folding/browser/folding.ts:550-565`), the pending `Delayer` computation started when the model is set (`folding.ts:271`, `:295-311`). `editor.trigger('', 'editor.foldAll')` right after `create`, `setModel` or `setValue` waits for the ranges, then folds them.
- **Port:** the commands run synchronously against `self.folds.ranges()` (`folding_commands.rs:39-54`, `fold_all` at `:133`). The ranges are first computed from the paint path: `sync_view_lines` → `compute_fold_ranges` (`editor.rs:5126-5136`, called from `element.rs:3450`), asynchronously (`editor.rs:5146-5230`). Before that, every fold command, `fold_with_args` and `execute_command("editor.foldAll")` is a no-op. `MONACO_PARITY.md:100` documents only that commands act on the moved regions at once instead of waiting for a pending recompute; it doesn't cover having no regions yet.
- **Repro:** `CodeEditor::new(...)` with a three-function Rust file, then `editor.fold_all(cx)` in the same update, then render: Monaco shows every function folded, the port none.
- **Fix:** while no ranges have been computed (or a computation is pending), queue the command and replay it when `compute_fold_ranges` finishes; or compute indentation ranges synchronously on demand.

#### D.2 Remove Manual Folding Ranges doesn't recompute, so the regions a manual range displaced stay gone (P3)
- **Monaco:** `RemoveFoldRangeFromSelectionAction` calls `foldingModel.removeManualRanges(selections)` and then `triggerFoldingModelChanged()` (`folding.ts:1206-1207`); the recompute brings back the regions `sanitizeAndMerge` had dropped for the user-defined range.
- **Port:** `remove_manual_folding_ranges` calls `folds.remove_manual` and `after_fold_command` only (`folding_commands.rs:317-334`); `remove_manual` keeps the remaining regions (`view/folding.rs:1415-1433`) and doesn't reset `ranges_version`, so nothing recomputes until the next edit, while `sanitize_and_merge` had dropped the replaced and crossed ranges (`view/folding.rs:914-916`, `:937-951`). `toggle_import_fold` does call `refresh_folding` (`folding_commands.rs:240-250`).
- **Repro:** `fn a() {\n  x\n}\nfn b() {\n  y\n}`. Select rows 0–4, Cmd/Ctrl+K Cmd/Ctrl+, (a manual fold replacing `a`'s region), then Cmd/Ctrl+K Cmd/Ctrl+. on it: Monaco's chevrons for `a` and `b` come back; the port's stay missing until you type.
- **Fix:** `self.refresh_folding(cx)` after a removal that changed anything.

#### D.3 A middle click on a collapsed line's `⋯` or its end doesn't unfold it (P3)
- **Monaco:** `onEditorMouseDown` accepts left and middle presses (`folding.ts:403`) on `CONTENT_TEXT` at the line's max column while ranges are hidden, and on `CONTENT_EMPTY` with `unfoldOnClickAfterEndOfLine`; the mouse-up toggles the collapsed region, recursively for a middle press (`:493`).
- **Port:** the middle-button handler only arms a press on a folding chevron (`editor.rs:5376-5383`, through `fold_control_at`); the `⋯` and line-end checks are on the left-button path only (`editor.rs:5722-5747`).
- **Repro:** collapse a function with a collapsed inner block, middle-click its `⋯`: Monaco unfolds both; the port column-selects or starts middle-scrolling.
- **Fix:** run the same placeholder and line-end checks in `on_middle_mouse_down` before the scroll and column-select handling.

### Go to definition (#487)

#### D.4 The Cmd/Ctrl+hover definition link is requested per mouse position, not per word, and flickers at the word's end (P3)
- **Monaco:** `startFindDefinition` returns early when the word under the mouse is the same as last time, found or not (`contrib/gotoSymbol/browser/link/goToDefinitionAtPosition.ts:142-145`), and asks the providers at once (`:157`).
- **Port:** `definition_mouse_move` keeps the link only while `position < range.end` (`definition.rs:184-189`), otherwise returns only for the exact same position (`:190`); `start_find_definition` clears the link first (`:206`) and asks again after 50ms (`:217`). Hovering the right half of a word's last character removes the underline and its hover, then shows them again; moving within a word without a definition asks the providers at every resting position.
- **Repro:** hold Cmd/Ctrl and move slowly across `foo`: Monaco's link stays steady; the port's underline and preview blink as the mouse crosses the middle of the last `o`, with one request per resting position.
- **Fix:** remember the last word (start, end, text) like `currentWordAtPosition`, return early for the same word, and drop the 50ms delay.

#### D.5 The go-to-definition target highlight has a solid border in high contrast themes (P3)
- **Monaco:** `.symbolHighlight` gets `border-style: dotted` under `.hc-black` and `.hc-light` (`browser/widget/codeEditor/editor.css:40-47`).
- **Port:** `highlight_symbol` sets the border but no `border_style` (`goto.rs:556-566`), unlike the word highlights' `dotted_in_high_contrast` (`occurrences.rs:157`, `:574`, `decorations.rs:489-495`); the multi diff editor's copy likewise (`multi_diff_editor.rs:2945-2946`).
- **Repro:** HC Black, F12 on a symbol defined in the same file: Monaco briefly shows a dotted box (dashed in the port's mapping), the port a solid one.
- **Fix:** `border_style: dotted_in_high_contrast(self.theme.high_contrast)` in both places.

### Widget focus (#472)

#### D.6 Bracket match highlights disappear when a widget inside the editor takes focus (P3)
- **Monaco:** `_recomputeBrackets` clears the brackets only without widget focus (`!this._editor.hasWidgetFocus()`, `contrib/bracketMatching/browser/bracketMatching.ts:338`) and reschedules on `onDidBlurEditorWidget`/`onDidFocusEditorWidget` (`:173-179`), so the pair stays highlighted while find, rename or a focused hover has focus.
- **Port:** `update_bracket_highlights` requires `self.has_focus` (`bracket_matching.rs:136`), which is text focus, cleared by `on_blur` of the editor's focus handle (`editor.rs:1003-1007`). `has_widget_focus` exists (`api.rs:1222`) and the word highlighter uses widget focus (`editor_commands.rs:383-391`).
- **Repro:** cursor after `)` in `foo(a, b)`, then Cmd/Ctrl+F: Monaco keeps both parentheses boxed, the port removes them.
- **Fix:** test widget focus (`focus_handle.contains_focused`) and recompute on widget blur, not text blur.

### Smart select (#493)

#### D.7 Expand Selection uses tree-sitter nodes instead of Monaco's bracket ranges, undocumented (P3)
Decide and document.
- **Monaco:** with no selection range provider, `provideSelectionRanges` uses `BracketSelectionRangeProvider` plus the word provider (`contrib/smartSelect/browser/smartSelect.ts:212-220`).
- **Port:** with no provider answer, it takes the syntax tree's `selection_ranges` first and falls back to brackets only without a tree (`smart_select.rs:148-175`), so every tree-sitter language expands through grammar nodes. `book/src/differences.md:64` documents the syntax-tree fallback for folding but not for smart select, and `MONACO_PARITY.md:55` lists smart select as plain ✅.
- **Repro:** Rust `let x = foo(a + b);`, cursor on `a`, Expand Selection repeatedly: Monaco goes `a` → `a + b` (bracket inside) → `(a + b)` → …; the port follows the tree (`a`, `a + b`, `(a + b)`, `foo(a + b)`, `let …;`).
- **Fix:** add the difference to `differences.md` and the parity row, or gate the syntax ranges behind an option.

## E. Diff editor and multi diff editor

Scope: diff rendering, hidden unchanged regions, the gutter and its toolbars, moved blocks, the accessible diff viewer, copying deleted lines, staged values, and the multi diff editor's file actions and options. Monaco paths starting `features/` and `components/` are under `browser/widget/diffEditor/`.

Checked and matching: the audit 4 §E fixes: the viewer's focused `Role::List` with the current row as active descendant (`diff_editor/accessible.rs:690-734`), groups rebuilt only on a new diff and closed by `set_model(None)`, screen reader mode in the automatic side-by-side key, `filterWithPrevious` and `clip`, the whole-file region kept, `_hide` and the compact fold glyph, high-contrast overview colors, dashed borders, the full-width tint, the render options and borders, gutter spans with zones, lanes and z-indexes, the modified overrides, the optional sash ratio, legacy empty inner changes, the aliases and the pending reveal reset, and the multi diff editor's pending view state, editor options, layout override, kept-file reset, collapsed bindings, 500px estimate, chevron focus, `focus_active` and the R flag.

### Diff editor (#480)

#### E.1 The staged value and the deleted lines' copy actions drop CRLF line endings (P2)
- **Monaco:** `computeStagedValue` applies the inner changes through `TextModelText`, whose `getValueOfRange` is `model.getValueInRange` (`features/gutterFeature.ts:173-180`, `common/model/textModelText.ts:16-17`), so the value uses the model's EOL. "Copy deleted lines" uses `getValueInRange` (`components/diffEditorViewZones/inlineDiffDeletedCodeMargin.ts:91-93`), "Copy deleted line (N)" gives an empty line the model's EOL (`:107-112`), and copying a selection uses `getValueInRange` (`components/diffEditorViewZones/copySelection.ts:72-73`).
- **Port:** `staged_value` (`diff_editor.rs:2461-2486`) builds the string from the ropes, which always store `\n` (only `TextModel::text` joins with `line_ending()`, `model.rs:880-889`). `DeletedCode::deleted_text` (`diff_editor.rs:1723-1734`) and `selected_text` (`:1701-1710`) copy rope text, and `deleted_line_text` (`:1737-1744`) hard-codes `"\n"`.
- **Repro:** both models with `\r\n` text, a `DiffHunkAction` that writes `original_with_modified_changes` to disk: Monaco's value has CRLF, the port's LF, so staging a hunk rewrites every line ending. Inline view, "Copy changed lines" on a three-line CRLF change: Monaco copies `a\r\nb\r\nc\r\n`, the port `a\nb\nc\n`.
- **Fix:** join with the model's `line_ending()` in `staged_value`, `deleted_text` and `selected_text`, and return it from `deleted_line_text`.

#### E.2 Cursor moves in an identical diff, and content flushes, reveal hidden lines (P3)
A side effect of audit 4 §E.6.
- **Monaco:** `ensureModifiedLineIsVisible`/`ensureOriginalLineIsVisible` return at once when the diff has no mappings (`diffEditorViewModel.ts:325-348`), so the whole-file region kept for an identical diff (`features/hideUnchangedRegionsFeature.ts:95-100` only hides it in the view) stays collapsed. The cursor listeners skip `CursorChangeReason.ContentFlush` (`hideUnchangedRegionsFeature.ts:73-75`, `:84-86`).
- **Port:** `ensure_selections_visible` (`diff_editor.rs:5396-5432`) runs whenever `unchanged_regions()` isn't empty, the whole-file region included, for every `SelectionsChanged` whatever the reason (`:2815-2816`; `CursorChangeReason::ContentFlush` exists, `events.rs:215-217`).
- **Repro:** two identical 100-line models, `hide_unchanged_regions` on. Click line 50, then type there: Monaco hides lines 1–46 and 54–100 behind bars after the diff; in the port the click set the region's `visible_top` to 50 and `transfer_state` carries it over, so lines 1–46 show with no bar.
- **Fix:** return early from `ensure_selections_visible` when the diff has no changes, and skip `ContentFlush`.

#### E.3 A compared moved block gets the regular diff decorations, not Monaco's compare set (P3)
- **Monaco:** while `movedTextToCompare` is set, only the line backgrounds of each change and `diffDeleteDecoration`/`diffAddDecoration` for every inner change are added: no whole-line `char-insert`/`char-delete` layers, no `diff-range-empty` bars, no in-range filter (`components/diffEditorDecorations.ts:86-101`).
- **Port:** the compared move's `changes` go through the same `diff_decorations` as the whole diff (`diff_editor.rs:3984-3995`, `:6846-6996`), adding the full-line text background for pure insertions or deletions (`:6939-6945`) and 3px empty-range bars (`:6950-6963`).
- **Repro:** `show_moves`, a moved block with one line inserted inside it; click Compare: Monaco gives the inserted line only `diffEditor.insertedLineBackground`, the port also `insertedTextBackground` over the whole line.
- **Fix:** a compare-specific builder: line backgrounds (with indicators) for non-empty ranges plus plain text decorations for every inner change.

#### E.4 Gutter toolbars drop actions past the limit instead of overflowing (P3)
- **Monaco:** each hunk and selection toolbar is a `MenuWorkbenchToolBar` with `primaryGroup: g => g.startsWith('primary')` and `overflowBehavior: { maxItems: isSmall ? 1 : 3 }` (`features/gutterFeature.ts:252-262`); extra and non-primary actions go into a "…" menu (`WorkbenchToolBar`, upstream, not vendored).
- **Port:** the action list is truncated to 3 (1 when small) and the rest are never shown (`diff_editor.rs:5956-5961`); `DiffHunkAction` (`:2435-2447`) has no group.
- **Repro:** `add_hunk_action` four times: Monaco shows the first ones and a "…" menu with the rest; the port shows three and the fourth is unreachable. On a small change only Revert shows, and every host action is unreachable.
- **Fix:** a "…" button opening a context menu with the truncated actions; optionally a `group` field for actions that go straight to the overflow.

#### E.5 The accessible diff viewer's rows ignore whitespace and control-character rendering (P3)
- **Monaco:** each row is rendered with `renderViewLine2` using the side's `renderWhitespace`, `renderControlCharacters`, `fontLigatures` and `stopRenderingLineAfter` (`components/accessibleDiffViewer.ts:658-689`).
- **Port:** rows are the line text after `expand_tabs` (`diff_editor/accessible.rs:579-583`), although `deleted_line_display` already renders whitespace for the inline deleted lines (`diff_editor.rs:1871-1891`).
- **Repro:** `render_whitespace: All`, a change adding trailing spaces, F7: Monaco's row shows `·` marks, the port's nothing.
- **Fix:** reuse `whitespace_columns` and `deleted_line_display` for viewer rows.

### Multi diff editor (#488)

#### E.6 Multi diff file actions have no `when`, toggled state or codicons, and `multiDiffEditorItemAllUnchangedRegionsShown` is missing (P3)
- **Monaco:** the file toolbar is a `MenuWorkbenchToolBar` on `MenuId.MultiDiffEditorFileToolbar` (`multiDiffEditor/diffEditorItemTemplate.ts:290-305`), in a context key scope with `multiDiffEditorItemAllUnchangedRegionsShown` (bound to `allUnchangedRegionsShown`, `diffEditor/diffEditorWidget.ts:121-124`) and the item's `contextKeys` (`:449-453`), with `highlightToggledItems: true`; actions render as codicons.
- **Port:** `MultiDiffFileAction` (`multi_diff_editor.rs:349-358`) has an id, label, icon text and handler. Every action shows on every file, the icon as plain text (`:2553-2575`), so `"$(fold)"` renders literally (`DiffHunkAction` goes through `label_with_icons`, `diff_editor.rs:5976`). The context key isn't set anywhere.
- **Repro:** a file action meant to toggle unchanged regions with icon `"$(unfold)"`: it shows the literal text and can't switch icon or state, or hide for some files.
- **Fix:** render the icon with `label_with_icons`; add optional `when` and `toggled` evaluated against a per-file context including `multiDiffEditorItemAllUnchangedRegionsShown` and the document's `context_keys`.

#### E.7 List-wide diff options can't override per-file options (P3)
Follows up audit 4 §E.23, whose layout override covers only three fields.
- **Monaco:** `_optionsOverride = { ...this._diffEditorOptions, ...diffLayoutOptions }` (`multiDiffEditor/multiDiffEditorWidgetImpl.ts:84-86`), where `_diffEditorOptions` comes from the widget's constructor (`multiDiffEditorWidget.ts:68-78`); `diffEditorItemTemplate.ts:372-377` spreads it after the file's own options, so widget-level options win.
- **Port:** the list's options apply only to documents without their own (`multi_diff_editor.rs:1278-1303`; `update_options` is documented that way, `:1306-1316`); only the layout trio and `hide_original_line_numbers` override a file's options.
- **Repro:** a document with `ignore_trim_whitespace: false`, then `update_options` on the list with `true`: with Monaco's widget-level options every file ignores trim whitespace; the port's file keeps showing whitespace changes.
- **Fix:** a list-level override layer merged after the file's options in `item_options`, or document the inversion in `differences.md`.

## F. Options, themes and languages

Scope: editor options and their validation, fonts and zoom, themes and token themes, language configuration (brackets, onEnter, indentation rules, word patterns), language registration and embedded languages.

Checked and matching: the audit 4 §F fixes: scroll anchoring on re-wrap, zoom and hidden-area changes (`editor.rs:1517-1524`, `:5040-5112`, `geometry.rs:234-261`), the unrounded zoomed line height (`font_zoom.rs:56-72`), Monarch's line length limit with embedded languages and the token provider exemption, the standard token type checks in bracket colorization and inline completions, injection layers re-resolved by id, one tokenizer factory slot per language and the documented disposal behavior. Audit 4 §F.2 is incomplete for the font size (§F.9).

### Embedded languages and bracket configuration (#481)

#### F.1 Typing inside an embedded language uses the outer language's onEnter rules, indentation rules and electric brackets (P2)
- **Monaco:** `getEnterAction` uses `getLanguageIdAtPosition` (`common/languages/enterAction.ts:20`). `getIndentForEnter` and `getIndentActionForType` use the language at the position and detect a line that mixes languages (`isLanguageDifferentFromLineStart`), where `getIndentActionForType` returns null (`common/languages/autoIndent.ts:304-330`, `:360-376`). `getPrecedingValidLine` stops at the embedded language's boundary (`autoIndent.ts:42-60`). The processed text is cut to the scoped language region (`supports/indentationLineProcessor.ts`), and `onElectricCharacter` uses `createScopedLineTokens(...).languageId` (`common/cursorCommon.ts:190-197`).
- **Port:** `with_language` always builds the `LanguageContext` from `model.language().config`, the top-level language (`editor.rs:4079-4104`), and every typing path uses it: `type_with_language`, Enter, `indent_for_enter`, `indent_action_for_type`, `on_electric_character` (`cursor/language.rs:885-1100`). `preceding_valid_line` has no language check (`cursor/language.rs:785-790`). The module doc says embedded languages aren't supported (`cursor/language.rs:15-17`), but neither `differences.md` nor `MONACO_PARITY.md` lists it. The model knows the embedded language: `word_at_position` and `language_id_at_position` use `line_language_runs` (`api.rs:213-285`), and comment toggling uses `language_at`.
- **Repro:** the bundled HTML (which injects JavaScript) with `<script>`, `  /** doc`, `</script>`; Enter at the end of line 2. Monaco applies JavaScript's onEnter rule and inserts `   * `; the port applies HTML's and keeps only the indent. The same goes for `}` and `)` outdenting and `if (x)` indenting inside `<script>` and `<style>`.
- **Fix:** build the `LanguageContext` per cursor from `model.language_at(offset)`; port `isLanguageDifferentFromLineStart` and the boundary stop in `getPrecedingValidLine`.

#### F.2 Bracket pairs use either `colorizedBracketPairs` or `brackets`, not both, and `<>` isn't dropped by default (P2)
- **Monaco:** `LanguageBracketsConfiguration` builds the bracket set from `brackets` plus `colorizedBracketPairs` (`common/languages/supports/languageBracketsConfiguration.ts:23-66`); when `colorizedBracketPairs` is unset, the colorized set is every bracket except `<`…`>` (`:51-56`). The bracket pairs tree pairs all of them; only colorization filters to colorized pairs (`closesColorized`, `bracketPairsTree.ts:342`), so `getBracketPairsInRange` and the bracket pair guides see every pair.
- **Port:** `refresh_bracket_colors` uses `colorized_bracket_pairs.unwrap_or_else(|| brackets())` for colors and guides (`bracket_colors.rs:44-50`) and returns early when that list is empty (`:52-57`); `TextModel::all_bracket_pairs` does the same (`api.rs:546-553`). Unset colorized pairs keep `<>`.
- **Repro:** a language with `brackets: [["<",">"],["(",")"]]` and no `colorizedBracketPairs`, text `if (a < b) {}`: Monaco colors `()` and leaves `<` alone; the port colors `<` as unmatched. The bundled HTML config (`colorizedBracketPairs: []`) with `guides.bracketPairs: true`: Monaco draws guides for multi-line `{ … }`; the port draws none and `bracket_pairs_in_range` is empty.
- **Fix:** pair over the union of `brackets` and the colorized pairs and color only the colorized ones, which default to `brackets` minus `<>`.

#### F.3 Explicitly empty `brackets` or `onEnterRules` count as unset for onEnter (P3)
- **Monaco:** `ResolvedLanguageConfiguration` creates an `OnEnterSupport` when `brackets || indentationRules || onEnterRules` is truthy, and `[]` is truthy (`common/languages/languageConfigurationRegistry.ts:373-378`); `OnEnterSupport` falls back to `()`, `{}` and `[]` only when `brackets` is undefined (`supports/onEnter.ts:29-34`).
- **Port:** `has_on_enter = !brackets().is_empty() || indentation_rules.is_some() || !on_enter_rules().is_empty()`, and the defaults apply whenever `brackets()` is empty (`cursor/language.rs:250-262`).
- **Repro:** `{ onEnterRules: [] }` on a language with no brackets, Enter after `foo {`: Monaco indents (default bracket rule), the port only copies the indentation. `{ brackets: [], indentationRules: {…} }`, Enter after `f(`: Monaco has no bracket rule, the port indents.
- **Fix:** base both checks on `is_some()`, with the defaults only when `brackets` is `None`.

#### F.4 Indentation rules strip only `brackets` from strings and comments, not the colorized pairs (P3)
- **Monaco:** `IndentationLineProcessor.getProcessedTokens` removes `bracketsNew.getBracketRegExp()` matches from string, comment and regex tokens (`supports/indentationLineProcessor.ts:202-220`); the regex covers `brackets` and `colorizedBracketPairs` (`languageBracketsConfiguration.ts:99-102`).
- **Port:** `bracket_regex` is built from `config.brackets()` only (`cursor/language.rs:284-301`).
- **Repro:** `brackets: [["{","}"]]`, `colorizedBracketPairs: [["(",")"]]`, `increaseIndentPattern: "\\([^)]*$"`; Enter after `x = "(";`: Monaco processes `x = "";` and doesn't indent; the port indents. The bundled TypeScript config has `<>` only among its colorized pairs, so `"<"` in a string differs the same way.
- **Fix:** build `bracket_regex` from the same union as §F.2.

### Language and theme definitions (#489)

#### F.5 Language configuration regexes use Rust's Unicode `\w`, `\d`, `\s` and `.` instead of JavaScript's (P3)
- **Monaco:** `wordPattern`, `indentationRules`, `onEnterRules` and `folding.markers` are JavaScript `RegExp`s; without `u`, `\w` and `\d` are ASCII and `.` excludes `\r`, U+2028 and U+2029.
- **Port:** `syntax::Pattern::new` passes the source through `translate_js` (`crates/syntax/src/config.rs:367-400`, `:437-500`), which only rewrites `[^]`, `[]`, `[` inside classes and hyphens next to class escapes; `\w`, `\d`, `\s` and `.` reach `fancy_regex` with Unicode semantics. The Monarch translator keeps JavaScript's sets (`monarch/js_regex.rs:1-24`, `:155-190`), and `differences.md:36` promises that for Monarch only; `book/src/languages.md:77-81` lists the config translations without these classes.
- **Repro:** `{ wordPattern: "\\w+" }` with text `naïve`: `word_at_position` at column 0 gives `na` in Monaco and `naïve` in the port, and so do occurrence highlights and word-based suggestions.
- **Fix:** run `Pattern` sources through `monarch::js_regex::translate`, or document the Unicode classes in `languages.md` and `differences.md`.

#### F.6 A language's `first_line` regex isn't anchored at the line start (P3)
- **Monaco:** `LanguagesRegistry._registerLanguage` prefixes `^` to a `firstLine` without one (`common/services/languagesRegistry.ts:218-226`); `getAssociationByFirstline` uses `firstLine.match(regex)` (`languagesAssociations.ts:245-263`).
- **Port:** `language_for_path_or_first_line` compiles `first_line` as written (no `^`, no JavaScript translation) and tests `is_match` anywhere in the line (`crates/syntax/src/language.rs:607-631`; stored raw by `with_first_line`, `:314-317`).
- **Repro:** a language `with_first_line("<\\?xml")` and a uri-less model whose first line is `foo <?xml`: Monaco doesn't guess the language, the port does.
- **Fix:** prefix `^` when missing and compile through the JavaScript translation.

#### F.7 A theme with `inherit: false` and no `editor.foreground` colors tokens with the editor foreground, not `#000000` (P3)
- **Monaco:** `StandaloneTheme.tokenTheme` starts from no rules when `inherit` is false and adds a `''` rule only when `editor.foreground` or `editor.background` is set (`standalone/browser/standaloneThemeService.ts:133-152`); `resolveParsedTokenThemeRules` then defaults the root foreground to `000000` (`common/languages/supports/tokenization.ts:119`), so `mtk1` is black even on a `vs-dark` base.
- **Port:** the root color defaults to none, meaning the editor foreground (`theme/token_theme.rs:10-13`, `default_style` in `theme.rs`), and `Theme::define` adds the `''` rule only from `editor.foreground` (`theme.rs:1969-1990`). The module comment says the result matches Monaco's, which doesn't hold here.
- **Repro:** `Theme::define("x", &Theme::vs_dark(), false, vec![], &HashMap::new())`, then `set_theme("x")`: Monaco draws `#000000` text on `#1E1E1E`; the port `#BBBBBB`.
- **Fix:** give a colorless root `#000000` when the theme has rules and no `''` rule, or list the difference in `differences.md` and correct the comment.

### Font options (#486)

#### F.8 Two-cell full-width characters ignore font ligatures; `font_info()`'s monospace test and fields differ (P3)
- **Monaco:** `isMonospace` starts as `fontFeatureSettings === EditorFontLigatures.OFF`, then compares `n`, space, the digits, `→`, `·` and `|/-_ilm%` in regular, italic and bold (`browser/config/fontMeasurements.ts:184-226`). `effectiveFullwidthCharacterWidth` is `twoCells` only when `isMonospace` (`common/config/editorOptions.ts:2122-2127`). `FontInfo` also exposes `maxDigitWidth`, `typicalFullwidthCharacterWidth`, `middotWidth`, `wsmiddotWidth`, `canUseHalfwidthRightwardsArrow`, `pixelRatio` and `fontFeatureSettings`.
- **Port:** `TextMetrics` turns two-cell mode on when `i`, `W`, `1`, `.`, `_` and space match `n`, whatever `font_ligatures` is (`element.rs:282-298`). `font_info().is_monospace` compares only space with `n` (`api.rs:2208-2218`). `FontInfo` lacks the fields above, though `max_digit_width` is measured (`element.rs:271-280`).
- **Repro:** `fullwidth_character_width: TwoCells`, `font_ligatures: On`, Menlo, `a中b`: Monaco uses the font's width for `中`, the port centers it in two cells. With ligatures off and a font whose `|` or `%` is narrower than `n`, Monaco reports `isMonospace: false` and the port `true`.
- **Fix:** one `is_monospace` (ligatures off plus Monaco's test set) shared by `TextMetrics` and `font_info`; add the missing `FontInfo` fields that have values here.

#### F.9 `computed_options().font_size` clamps the zoomed size to 6–100px (P3)
Incomplete fix of audit 4 §F.2.
- **Monaco:** `EditorFontSize.compute` returns `env.fontInfo.fontSize`, the validated size times `1 + zoomLevel * 0.1`, unclamped (`common/config/editorOptions.ts:2134-2160`, `fontInfo.ts:42-50`).
- **Port:** `zoomed_options` multiplies the validated size (`font_zoom.rs:56-63`) and rendering uses it unclamped (`element.rs:257`), but `computed_options` sets `font_size = options.resolved_font_size()`, which clamps again (`editor.rs:8303`, `options.rs:2495-2503`); `ConfigurationChanged` carries the same value.
- **Repro:** `font_size: 80`, zoom level 5: Monaco reports 120 and renders at 120px; the port renders at 120px but reports 100. Zoom level −5 with `font_size: 10`: Monaco 5, the port 6.
- **Fix:** `computed.font_size = options.font_size` (already validated and zoomed), as for the line height.

#### F.10 `FontLigatures::Settings` doesn't normalize `""`, `"false"` and `"true"` (P3)
- **Monaco:** `EditorFontLigatures.validate` maps `'false'` or `''` to off (`"liga" off, "calt" off`) and `'true'` to on; other strings are kept (`common/config/editorOptions.ts:1948-1965`).
- **Port:** `EditorOptions::validated` doesn't touch `font_ligatures` (`options.rs:2558-2660`), and `parse_font_feature_settings` drops unquoted entries (`options.rs:1747-1767`), so all three give no features and the font's default ligatures apply.
- **Repro:** `FontLigatures::Settings("false".into())` with Fira Code, `=>`: Monaco renders two glyphs, the port the ligature.
- **Fix:** in `validated()`, map `Settings("" | "false")` to `Off` and `Settings("true")` to `On`.

## G. Public API and the LSP bridge

Scope: `monaco.editor.*` and `monaco.languages.*` equivalents, the `ICodeEditor`/`ITextModel` surface and events, decorations, commands, provider contexts, uris and the model registry, `crates/gpui-monaco-lsp`, and the book's API pages. The LSP bridge is compared with `monaco-languageclient` and `vscode-languageclient` (upstream, not vendored).

Checked and matching: the audit 4 §G fixes: the compact re-query (`goto.rs:181-222`), `ProviderTarget` on `resource()` with `plaintext`, eager language resolution, canonical ids, `run_provider_command`, `-command` removals, the links key, inlay hint and code lens epochs, the formatting `indent_size`, semantic token score groups, `replace_range` validation, `provide_signature_help_with_context`, the word range as the default hover range, the shared `get_links`, openers falling back to `open_url`, `InlineCompletionCommand.icon`, and the `migrating.md` rows.

### LSP bridge uris and edits (#471)

#### G.1 The LSP bridge sends the raw uri and matches server uris by raw string: paths with spaces or non-ASCII can't attach, and normalized server uris lose diagnostics and edits (P1)
- **Monaco:** `monaco-languageclient` identifies documents by `model.uri.toString()`, the normalized form (`file:///a b.rs` becomes `file:///a%20b.rs`, `file:///C:/x` becomes `file:///c%3A/x`), and parses incoming uris back into `URI`s (upstream, not vendored). The port has this form as `TextModel::resource()` and `normalize_uri` (`model.rs:837-846`, `uri.rs:26-31`).
- **Port:** `attach` keys `documents` by the raw `model.uri()` (`crates/gpui-monaco-lsp/src/client.rs:238-241`) and sends it through `lsp::Uri::from_str` (`client.rs:382`). `lsp-types` 0.97 uses `fluent-uri` 0.1.4, which enforces RFC 3986, so a space or non-ASCII character fails `attach` with "invalid uri"; `prepare_request` returns `None` for any uri that doesn't parse (`client.rs:358`). `publishDiagnostics` and `workspace/applyEdit` look models up by exact string (`client.rs:538-541`, `:676-679`), so a server that normalizes uris (Node servers using `vscode-uri` turn `C:` into `c%3A`) publishes to a key that isn't there, and its diagnostics are dropped. A model without a uri can't be attached (`client.rs:240`), where Monaco would send `inmemory://model/N`.
- **Repro:** `TextModel::new(src).with_language("rust").with_uri("file:///home/me/my project/src/main.rs")`, then `client.attach(&model, cx)`: `Err("invalid uri …")` and no features. On Windows with `typescript-language-server` and `file:///C:/proj/a.ts`: hovers work, but diagnostics published for `file:///c%3A/proj/a.ts` never become markers.
- **Fix:** key `documents` by `model.resource()`, send that to the server, and run every incoming uri through `gpui_monaco::normalize_uri` before lookups (diagnostics, `applyEdit`, and `convert::locations`/`inlay_hint`'s `queried` comparison).

#### G.2 `workspace/applyEdit` bypasses the bulk-edit path: it fails on file operations and unattached models, and always reports `applied: true` (P3)
- **Monaco:** `workspace/applyEdit` goes to the bulk edit service; standalone `StandaloneBulkEditService` finds models through the model service, fails with "bad edit - model not found" otherwise, checks `versionId` and applies one undo step (`standalone/browser/standaloneServices.ts:821-864`). The port's `CodeEditor::apply_workspace_edit` (`bulk_edit.rs:193`) does this, with file operations going to `set_workspace_file_edit_handler`, and the bridge's rename already uses it through `convert::workspace_edit` (`crates/gpui-monaco-lsp/src/convert.rs:499`).
- **Port:** `LspClient::apply_workspace_edit` (`crates/gpui-monaco-lsp/src/client.rs:637-700`) fails on any `DocumentChangeOperation::Op` (`:652`) and on a document that isn't attached even when `model_for_uri` has it (`:680`). It ignores change annotations and versions, applies each document with `model.edit(...).log_err()` (`:699`) and returns `applied: true` even when an edit errors (`:703`).
- **Repro:** a code action whose command makes the server send `workspace/applyEdit` with a `CreateFile`, or with edits to a registered but unattached model: it fails, though a rename producing the same edit works. An edit with an out-of-range position is logged, but the server is told it applied.
- **Fix:** convert with `convert::workspace_edit` and apply through the core bulk-edit service (exposed at the `App` level), returning its error as `failureReason`.

#### G.3 LSP diagnostics for documents that aren't attached are dropped (P3)
- **Monaco:** `vscode-languageclient` writes `publishDiagnostics` into a diagnostic collection keyed by uri, which reaches the marker service whether or not a model is open (upstream, not vendored); `getModelMarkers({ owner })` lists them, and a model opened later is decorated at once.
- **Port:** the marker service is keyed by uri and outlives models (`markers.rs:389-412`), but the only way in is `TextModel::set_markers`, and the bridge returns early when no attached model has the uri (`crates/gpui-monaco-lsp/src/client.rs:538-544`).
- **Repro:** `rust-analyzer` publishes workspace-wide diagnostics after `cargo check`: `model_markers` with the bridge's owner lists only attached files, and opening another file shows no squiggles until the server republishes.
- **Fix:** expose a uri-based `set_markers_for_uri(cx, owner, uri, markers)` and use it in the bridge.

#### G.4 LSP code action requests send numeric diagnostic codes back as strings and drop related information (P3)
- **Monaco:** `vscode-languageclient`'s `asDiagnostic` keeps the server's code type and related information, so `CodeActionContext.diagnostics` round-trips what the server published (upstream, not vendored).
- **Port:** `convert::marker` turns `NumberOrString::Number(n)` into a string (`crates/gpui-monaco-lsp/src/convert.rs:462-465`); `convert::diagnostic` always sends `NumberOrString::String` and leaves `related_information` unset (`convert.rs:634-637`, `:655`).
- **Repro:** the server publishes `{ code: 2304, … }`, then quick fixes are requested: the request carries `"code": "2304"`, and a server matching diagnostics by deep equality or numeric code finds no fix.
- **Fix:** keep the original `lsp::Diagnostic` (in a marker field or a side table the bridge owns) and send it back unchanged.

### LSP bridge features (#482)

#### G.5 The LSP bridge doesn't bridge code lenses, document links, document colors or linked editing (P2)
- **Monaco:** `monaco-languageclient` registers providers for `codeLensProvider` (with resolve and `workspace/codeLens/refresh`), `documentLinkProvider` (with resolve), `colorProvider` and `linkedEditingRangeProvider` (upstream, not vendored); the editor consumes all four (`registerCodeLensProvider`, `registerLinkProvider`, `registerColorProvider`, `registerLinkedEditingRangeProvider`, `standalone/browser/standaloneLanguages.ts:531`, `:563`, `:614`, `:630`).
- **Port:** `LspFeatures::register` registers completion, hover, the definition family, code actions, folding, selection ranges, semantic tokens, document symbols, inlay hints, rename, references, highlights, formatting and signature help (`crates/gpui-monaco-lsp/src/features.rs:175-353`), none of the four. The client capabilities don't advertise them (`client.rs:743-822`) and there's no `workspace/codeLens/refresh` handler (`client.rs:591-630`). `MONACO_PARITY.md:235` marks the bridge ✅.
- **Repro:** `rust-analyzer` ("Run | Debug", "N references" lenses) or `vscode-html-language-server` (linked editing, document links) through the bridge: Monaco with `monaco-languageclient` shows the lenses, links and tag-pair renaming; the port none.
- **Fix:** implement `CodeLensProvider`, `LinkProvider`, `DocumentColorProvider` and `LinkedEditingRangeProvider` on `LspFeatures`, advertise the capabilities, and map `workspace/codeLens/refresh` to `refresh_code_lenses`.

### Model uris in the API (#490)

#### G.6 `editor.action.goToLocations`, `peekLocations` and `showReferences` can't target a model without a uri, and compare uris raw (P3)
- **Monaco:** the commands take `model.uri`, `inmemory://model/N` for a model created without one; `openCodeEditor` matches `model.uri.toString() === resource.toString()` (`contrib/gotoSymbol/browser/goToCommands.ts:781-789`, `standalone/browser/standaloneCodeEditorService.ts:96-98`).
- **Port:** `editor_for_uri` skips models whose `uri()` is `None` (`goto.rs:716-723`), so `inmemory://model/1` never matches. `go_to_locations_action` then compares `args.uri` with the raw `uri()` (`goto.rs:626-629`), although its caller matched with the normalizing `is_at`, so `file:///a%20b.rs` reaches the editor of `file:///a b.rs` and is rejected. `check_symbol_navigation` (`goto.rs:622`) and `sorted_locations`' dedup (`goto.rs:115-125`) compare raw strings too.
- **Repro:** `create_model(cx, "x\nx", None, None)` in an editor, then `execute_command` with `editor.action.peekLocations` and `model.resource()`: Monaco peeks, the port does nothing.
- **Fix:** match with `is_at` against `resource()` in those three places, and normalize before sorting and deduplicating.

#### G.7 `ModelChange.old_uri`/`new_uri` are the raw uri, `None` for a model without one (P3)
- **Monaco:** `onWillChangeModel`/`onDidChangeModel` carry `this._modelData?.model.uri || null` and `model?.uri || null` (`browser/widget/codeEditor/codeEditorWidget.ts:521-522`): normalized, `inmemory://model/N` when none was given, `null` only without a model.
- **Port:** `set_model_inner` fills them from `uri().cloned()` (`editor.rs:1295-1298`), so switching between two uri-less models reports `None → None`, and `file:///a b` is unnormalized. The docs map them one to one (`events.rs:353-358`, `book/src/migrating.md`).
- **Repro:** an editor on `create_model(cx, "a", None, None)`, then `set_model` to another uri-less model: Monaco reports `inmemory://model/1 → inmemory://model/2`, the port `None → None`.
- **Fix:** use `resource()` for real models and `None` only for the detached placeholder.

#### G.8 `ProviderContext.uri` is the raw uri and `None` for uri-less models, though selectors score `resource()` (P3)
- **Monaco:** every provider gets `model`, whose `uri` is always set and normalized.
- **Port:** `provider_context()` passes `self.uri.clone()` (`model.rs:858-864`), documented as possibly `None` (`providers.rs:194-197`), while `ProviderTarget` uses `resource()` (`providers.rs:119-126`); a provider selected by `{ scheme: "inmemory" }` is then asked with `uri: None`.
- **Repro:** a definition provider for `{ scheme: "inmemory" }` returning `Location::new(Some(context.uri.clone()?), range)` is selected for a uri-less model but can't name it. A model `with_uri("file:///a b")` gives the provider `file:///a b`, while `model_for_uri`, markers and `ModelMarker.resource` use `file:///a%20b`.
- **Fix:** add `resource` to `ProviderContext` (or make `uri` the resource).

### Decorations (#483)

#### G.9 Decorations lack `hideInCommentTokens` and `hideInStringTokens` (P3)
- **Monaco:** both are public `IModelDecorationOptions` (`common/model.ts:297-303`); the view model drops such decorations when their range lies in a comment or string token (`common/viewModel/viewModelDecoration.ts:23-27`).
- **Port:** `DecorationOptions` has neither (`decorations.rs:330-485`). The unicode highlighter filters by itself (`unicode_highlighter.rs:115`, `:312`), and neither `book/src/decorations.md` nor `differences.md` mentions the gap.
- **Repro:** a background decoration over a range inside a comment that should hide there: the port can't express it and always draws it.
- **Fix:** add `hide_in_comment_tokens`/`hide_in_string_tokens` and filter in the view's decoration collection with the line's standard token types.

### Docs (#493)

#### G.10 `differences.md` says edits and decorations take byte ranges (P3)
Decide and document.
- **Monaco:** n/a (doc accuracy).
- **Port:** `book/src/differences.md:8` says edits and decorations "take byte offset ranges, not `IRange`", but the editor's `execute_edits`, `execute_edits_from`, `create_decorations_collection` and `delta_decorations` (`api.rs:1230`, `:1245`, `:1780`, `:2071`) and the model's `apply_edits` and `delta_decorations_at` (`api.rs:762`, `:2012`) take `Range<PointUtf16>`, as `migrating.md` shows; only `TextModel::edit` and `TextModel::delta_decorations` are byte-based (`model.rs:1611`, `:1714`).
- **Repro:** read `differences.md`, then `migrating.md`'s "Editor" table: they contradict each other.
- **Fix:** say ranges are 0-based `Range<PointUtf16>`, with byte-offset variants on `TextModel`.

#### G.11 The LSP docs list five features where the bridge registers about twenty, and don't say what `applyEdit` can't do (P3)
Decide and document.
- **Monaco:** n/a (doc accuracy).
- **Port:** `book/src/lsp.md:5`, the crate docs (`crates/gpui-monaco-lsp/src/lib.rs:4-7`) and `LspClient`'s doc comment (`client.rs:82-84`) name completion, hover, go to definition, document highlights and signature help; `lsp.md`'s own table (`:52-68`) and `features.rs:175-353` cover many more. `lsp.md:75` says `workspace/applyEdit` applies to attached models without saying that file operations fail or that errors still report success (§G.2).
- **Repro:** read `lsp.md`'s first section next to its Features table.
- **Fix:** replace the five-feature lists with a pointer to the table, and document the `applyEdit` limits (or fix §G.2).

## H. Input, accessibility and keybindings

Scope: default keys and precedence, host actions and keybinding rules, context keys, mouse handling, middle scroll, drag and drop and external drops, the accessibility tree, tab focus mode and `alert`/`status` calls.

Checked and matching: the audit 4 §H fixes: widget roles and names (find, rename, quick input, menus, the references tree, sticky scroll), the editor's role description and `aria-autocomplete`, the paste chip after `close_find` in the Escape chain (`editor.rs:3992-3996`), focus only for text and line numbers on left and right clicks (`editor.rs:5604-5622`), the documented Insert key, and `-command` removals through `gpui::Unbind`. Also the per-platform key tables (spot-checked), `SimplePagedScreenReaderStrategy` paging, screen reader computed options, tab focus mode, `editorSelectionHasDiagnostics` and every vendored `alert`/`status` call. Audit 4 §H.1 and §H.4 are incomplete (§H.10, §H.3, §H.7).

### Widget focus (#472)

#### H.1 Host actions and keybinding rules run in the find, rename or quick input box instead of the editor (P2)
- **Monaco:** the find, rename and quick inputs are plain `<input>`s. Keybindings from `addEditorAction` (`standalone/browser/standaloneEditor.ts:135-148`) and `addKeybindingRules` (`:204-218`) resolve their editor through `getFocusedCodeEditor() || getActiveCodeEditor()` (`browser/editorExtensions.ts:311`), and `getFocusedCodeEditor` returns the editor with *widget* focus when none has text focus (`browser/services/abstractCodeEditorService.ts:104-120`). A host action pressed in the find input runs on the find widget's editor, where `editorFocus` is true and `editorTextFocus` false (`codeEditorWidget.ts:2285-2287`), so a `when: 'editorTextFocus'` rule doesn't fire.
- **Port:** widget inputs are `CodeEditor`s (`editor.rs:945-964`) whose key context carries `CodeEditor`, `editorTextFocus`, `textInputFocus` and `editorFocus` when focused (`context_keys.rs:223-225`; only `editorSimpleInput` tells them apart). `add_editor_action` binds `RunAction` under `CodeEditor` (`editor_actions.rs:834-836`, `:855-875`) and `add_keybinding_rules` binds `RunCommand` under `CodeEditor` or the host's `when` (`:964-990`). GPUI dispatches to the focused widget input first, whose own listeners (`editor.rs:7895-7904`) run the action with `self` = the widget input (`editor_actions.rs:1249-1266`, `:1340-1400`). The find widget redirects only the built-in multi-cursor actions (`find_widget.rs:1811-1835`).
- **Repro:** `add_editor_action` with `cmd-u` uppercasing the selection (or `add_keybinding_rule(cx, KeybindingRule::new("ctrl-shift-u", "editor.action.transformToUppercase"))`). Open find, type `abc` in the find input, press the key: Monaco uppercases the document selection; the port uppercases the search string. The same happens in the rename box and the quick input.
- **Fix:** in the `RunAction`/`RunCommand` listeners, `cx.propagate()` for widget inputs so the owning editor handles them, and set `editorTextFocus`/`editorFocus` false for widget inputs, keying the built-in bindings that must work inside them on `textInputFocus`.

#### H.2 A focused sticky line keeps `editorTextFocus`, so modified shortcuts still edit the text (P3)
- **Monaco:** `StickyScrollController.focus` moves DOM focus to the sticky line (`contrib/stickyScroll/browser/stickyScrollController.ts:182-197`), so `editorTextFocus` turns false (`codeEditorWidget.ts:2286`) and every `editorTextFocus` rule stops applying.
- **Port:** `focus_sticky_scroll` only sets `sticky.focused` (`sticky_model.rs:668-674`); the editor keeps focus and `editorTextFocus` (`context_keys.rs:223`). `sticky_swallows` drops plain and Shift keys but lets Ctrl, Alt and Cmd keystrokes through (`sticky_model.rs:322-336`, used at `editor.rs:1052-1058`) to the text bindings. `MONACO_PARITY.md:107` documents only the plain-key part.
- **Repro:** sticky lines showing, `editor.action.focusStickyScroll`, then Cmd/Ctrl+/ or Cmd/Ctrl+Shift+K or Alt+Down: Monaco does nothing; the port comments, deletes or moves the cursor's line.
- **Fix:** clear `editorTextFocus` and `textInputFocus` (keeping `editorFocus`) while a sticky line is focused.

### Mouse and drag and drop (#491)

#### H.3 Middle-clicking the minimap, scrollbars or gutter focuses the editor and starts a column selection (P3)
Incomplete fix of audit 4 §H.4.
- **Monaco:** `MouseHandler._onMouseDown` focuses and starts the mouse-down operation for the middle button only on content targets (or line numbers with `selectOnLineNumbers`); gutter targets `preventDefault`, scrollbar and minimap targets match no branch (`browser/controller/mouseHandler.ts:310-347`), so there's no focus and no `_columnSelect` (`browser/view/viewController.ts:222-227`). With `scrollOnMiddleClick`, focus comes first for text targets, then `MiddleScrollController` starts (`contrib/middleScroll/browser/middleScrollController.ts:49-60`), so its key-down can end the session (`:85-87`).
- **Port:** `on_middle_mouse_down` (`editor.rs:5351-5428`) excludes only widgets and chevrons; for every other target it calls `window.focus` (`:5390`) and dispatches a column-select gesture (`:5418-5427`). With `scroll_on_middle_click` it starts scrolling and returns before focusing (`:5384-5388`), so the end-on-key-down listener (`editor.rs:8234-8236`) never sees keys when the editor wasn't focused.
- **Repro:** two editors, type in A, middle-click B's minimap or scrollbar: Monaco keeps focus in A; the port focuses B and puts a cursor at the mapped position. B's line numbers: the port column-selects from column 1. With `scroll_on_middle_click`, middle-click B's text while A is focused: Monaco focuses B, the port doesn't.
- **Fix:** apply `on_mouse_down`'s target test to the middle button, and focus before starting middle scroll on content targets.

#### H.4 Dragging files over a read-only editor shows the drop caret (P3)
- **Monaco:** `isDropIntoEnabled` is `!readOnly && dropIntoEditor.enabled`, and `onDragOver` returns without `showDropIndicatorAt` when it's false (`browser/widget/codeEditor/codeEditorWidget.ts:368-382`).
- **Port:** the `on_drag_move::<ExternalPaths>` listener checks only `drop_into_editor.enabled` (`editor.rs:7988-7999`); `drop_data` then ignores the drop (`drop_paste.rs:543-545`).
- **Repro:** `read_only: true`, drag a file over the text: Monaco shows nothing; the port draws the drop caret, then drops nothing.
- **Fix:** add `!read_only` to the drag-move check.

#### H.5 The drag-and-drop pointer stays an I-beam while the drag is over its own selection (P3)
- **Monaco:** from the first drag step, `_onEditorMouseDrag` sets `mouseStyle` to `'copy'` with the trigger modifier, else `'default'`, wherever the mouse is (`contrib/dnd/browser/dnd.ts:127-139`).
- **Port:** `root_mouse_cursor` shows the arrow only once `dnd.target` is set or copying (`editor.rs:6406-6413`), and `dnd_mouse_drag` clears `target` inside the selection (`dnd.rs:170-177`).
- **Repro:** select a word, press on it and drag a few pixels within it: Monaco shows the arrow, the port the I-beam until the mouse leaves the selection.
- **Fix:** test for a started drag (`dnd.selection.is_some()`) in `root_mouse_cursor`.

#### H.6 The copy modifier survives a blur, so a later drag copies instead of moving (P3)
- **Monaco:** `onEditorBlur` clears the drag selection, `_mouseDown` and `_modifierPressed` (`contrib/dnd/browser/dnd.ts:58-70`); the drop copies when `hasTriggerModifier(event) || this._modifierPressed` (`:180-192`).
- **Port:** the blur handler calls `dnd_cancel` (`editor.rs:1005-1020`), which returns early without an active drag and otherwise keeps `modifier_pressed` (`dnd.rs:258-268`); `dnd_mouse_down` keeps it too (`dnd.rs:144-148`), and the drop copies on it (`dnd.rs:234`).
- **Repro (macOS):** hold Alt in the editor, Cmd+Tab away so the key-up goes elsewhere, come back, drag a selection without Alt: Monaco moves it, the port copies it.
- **Fix:** reset `modifier_pressed` in the blur handler.

#### H.7 With `context_menu: false`, right-clicking inside a secondary selection keeps every cursor (P3)
Incomplete fix of audit 4 §H.4.
- **Monaco:** with `contextmenu` off, `_onContextMenu` focuses and calls `setPosition(target.position)` unless the *primary* selection contains it (`contrib/contextmenu/browser/contextmenu.ts:85-91`); only the menu-on path checks every selection (`:116-129`).
- **Port:** both paths use `context_menu_mouse_down`, which keeps the cursors when any selection contains the position (`context_menu.rs:1068-1083`, called at `editor.rs:5519-5520`).
- **Repro:** `context_menu: false`, two selections, right-click inside the secondary one: Monaco ends with one cursor at the click; the port keeps both selections.
- **Fix:** test only the primary selection when `context_menu` is off.

#### H.8 The drag-and-drop target line uses the cursor color, not Monaco's fixed `dnd-target` colors (P3)
- **Monaco:** `.dnd-target` is a 2px dotted right border, black in vs and hc-light, `#AEAFAD` in vs-dark, `#fff` in hc-black (`contrib/dnd/browser/dnd.css:6-18`); the external drop indicator uses the same class (`codeEditorWidget.ts:72-75`).
- **Port:** the dashes are painted in `colors.cursor` (`element.rs:6633-6643`), `editorCursor.foreground`: `#0F4A85` in hc-light (`theme.rs:793`) or whatever a theme sets.
- **Repro:** hc-light, or a theme with `editorCursor.foreground: #ff0000`, drag a selection: Monaco's target line is black, the port's blue (red).
- **Fix:** a `dnd_target` color from the theme type, independent of the cursor color.

### Accessibility (#492)

#### H.9 Read-only editors are announced as read-only; Monaco does so only with `domReadOnly` (P3)
- **Monaco:** the textarea gets `readonly` only when IME is disabled or `domReadOnly && readOnly` (`browser/controller/editContext/textArea/textAreaEditContext.ts:679-689`); `domReadOnly` (default false) is public (`common/config/editorOptions.ts:176-188`, `:6317`). A plain read-only editor is an editable textbox with `aria-autocomplete` "none".
- **Port:** there's no `dom_read_only`, and the accessibility builder sets `node.set_read_only()` whenever `read_only` (`editor.rs:7308-7309`).
- **Repro:** `read_only: true`, VoiceOver: Monaco reads "Editor content, editor"; the port adds "read only".
- **Fix:** add `EditorOptions::dom_read_only` and set the read-only flag only for `dom_read_only && read_only`, or document the difference.

#### H.10 The suggest list has no listbox or option roles; the focused suggestion is an assertive alert instead of the active descendant (P3)
Follows up audit 4 §H.1, which didn't cover suggest.
- **Monaco:** the suggest list is a `listbox` labelled "Suggest" with `option` rows (`listitem` on Windows) labelled by `getAriaLabel` (`contrib/suggest/browser/suggestWidget.ts:240-244`); the textarea's `aria-activedescendant` points at the focused row once it resolves (`:452`) and is cleared when nothing is focused (`:398`), so it is read politely as the combobox's selection.
- **Port:** `suggest_widget.rs` and `suggest/` set no roles or labels; `announce_suggest_widget` pushes the focused item's label through the assertive `announce` (`completion.rs:1534-1556`, `suggest_widget.rs:1054-1065`), interrupting the reader on every arrow key. `differences.md:65` lists the widgets that got roles and doesn't mention suggest.
- **Repro:** VoiceOver or an AccessKit inspector, trigger suggest and arrow down: Monaco exposes a "Suggest" listbox and reads the option as the active descendant; the port has no list node and alerts every row.
- **Fix:** `Role::ListBox` labelled "Suggest" with `ListBoxOption` rows and labels, and the root's `set_active_descendant` on the focused row (as the editor already does, `editor.rs:7322`), or document the alert fallback.
