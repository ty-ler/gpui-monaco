# Monaco parity audit 3 (2026-10-12)

A third audit, done after every issue from [`MONACO_AUDIT_2.md`](MONACO_AUDIT_2.md) was fixed. Eight read-only audits compared the port at `cd76117` with the vendored Monaco source, one per area: editing and cursors, view rendering, widgets and menus, navigation and scrolling, the diff editors, options and themes and languages, the public API, and input, accessibility and keybindings. Each finding was checked by reading both sides. Rendering findings B.1, B.4 and B.5 were also confirmed with playground screenshots, and the diff fixtures and diff editor tests were run. Mouse drags and screen readers weren't scripted, so those findings come from reading the code.

The audits overlapped in places. Each shared finding is listed once below, in the section that fits it best, and says which other audit found it too. Left out:

- what [`book/src/differences.md`](../book/src/differences.md) documents as a deliberate difference with a reason that holds (updated by #304);
- tracked work: injected text in view columns (#298), the GPUI-blocked items (#231–#233, #305–#309) and publishing to crates.io (#57);
- findings already fixed on `main`, including the stale doc lines #304 corrected.

`vs/platform` and most of `vs/base/browser` aren't vendored (`aria.ts`, `quickInputActions.ts`, `scrollableElement.ts`, the hover widget and list styles among them). Findings that rely on that code say *upstream, not vendored* and are worked from VS Code as it ships.

Paths: the port's are relative to `crates/gpui-monaco/src/` unless they start with `crates/`, `book/` or `docs/`. Monaco's are relative to `vendor/vscode/src/vs/editor/` unless they start with `base/`.

Severity: **P1** visibly broken or wrong behavior; **P2** a missing Monaco sub-feature or a notable fidelity gap; **P3** polish.

## Issues

| Issue | Scope | Sections |
|---|---|---|
| #323 | Reveal and autoscroll: only reveal where Monaco does, reveal all cursors, PageUp/PageDown | D, G |
| #324 | Folding state follows the cursor and the text: unfold on reveal, ranges that move with edits | D |
| #325 | Off-screen rename and code action menu, lightbulb placement and code action gaps | C |
| #326 | Language configuration without a grammar, plain text, late registration and MIME types | F, G |
| #327 | View lines: tab stops after wide characters, scroll width with word wrap, guides on wrapped lines | B |
| #328 | Line and bracket commands: in-place replace reversed, Go to Bracket, trim whitespace, tab-size picker | A, F |
| #329 | Gutter and decoration hovers: whole-line hovers, placement, lanes, line-number and markdown messages | C |
| #330 | F8 marker navigation: hints, order, marker changes and reveal | C |
| #331 | Typing: doc-comment alignment, IME commits, snippet closers, Insert Line and auto-closed tracking | A |
| #332 | Tab and Shift+Tab: quick input and rename leak, tab focus mode, read-only editors, snippet choices | C, H |
| #333 | Cursor movement: word moves under word wrap, side edits, sticky tab stops, viewport targets | A |
| #334 | Multi-cursor: Cmd+D case matching, column select across folds, undo stops | A |
| #335 | Content hover: dismissal on keys, clicks, mouse-leave and wheel, and hover stability | C |
| #336 | Keybinding precedence: ghost-text accept word, quick input keys, Escape order, Menu key | H |
| #337 | Sticky scroll and section headers: stale state after set_model, clicks, pointer and focus | D |
| #338 | Diff editor: normalized diffs, revert ranges, moved-block compare, alignment and scroll stability | E |
| #339 | Folding: recompute triggers, imports by default, range kinds and polish | D |
| #340 | Find widget: options feedback, find-in-selection scope, reveal, escaping and colors | C |
| #341 | Inline toolbar and menus, paste and drop chip and progress, peek title | C |
| #342 | Markers: hint dots, high-contrast borders, uri-less models, lifetime and ranges | F, G |
| #343 | Quick input, rename, context menu and color picker fidelity | C |
| #344 | Suggest widget and parameter hints: anchoring, details while resolving, Tab-completion follow-ups | C |
| #345 | Decorations overlay: showIfCollapsed on find and unicode highlights, empty-range slivers, layering | B |
| #346 | Multi diff editor: incremental document updates, first-change selection, placeholder, per-file options | E |
| #347 | Accessibility: screen reader page window, missing announcements, polite status messages | H |
| #348 | Options: standalone defaults, computed options, theme defaults and doc fixes | F, G |
| #349 | Editor API: cursor-change sources, decoration events, ids, action descriptors and missing events | G |
| #350 | Go to definition link and navigation polish: mouse-up clicks, link color, F12 highlight, selection anchor | D, H |
| #351 | Clipboard, format and snippet edits: block paste, copy EOL, force-moved markers, snippet overwrite | A |
| #352 | Diff editor: hidden unchanged regions merge, drag and scroll anchor | E |
| #353 | Model API: markdown links through openers, uri normalization, edit helpers, word at position | G |
| #354 | Diff editor: context keys, content size event and polish | E |
| #355 | Scroll command polish: scrollEditorBottom, smooth scroll commands, revealLine, composed animation | D, G |
| #356 | Input: trackpad pinch zoom, press-and-hold at every cursor, touch docs | H |
| #357 | Port the missing Monaco editing test suites | A |

Several findings share a root cause and are filed together:

- **`cursors_changed(true)` always requests an autoscroll** (§G.1, §D.16, §D.17), and the autoscroll reveals only the primary cursor (§D.18). #323 covers all four.
- **The language configuration is dropped whenever there's no grammar** (§F.1). Plain text (§F.2) is the most common case. Both are in #326.
- **Contextual keybindings in the main list lose to plain bindings in the platform blocks** (§H.1, §H.2, #336). See the note at the start of §H.
- **Off-screen anchors aren't revealed before a widget is shown:** rename and the code action menu (§C.22, §C.23, #325) and the marker zone (§C.13, #330).

## Counts

204 findings after merging: 22 P1, 65 P2 and 117 P3.

| Section | Findings | P1 | P2 | P3 |
|---|---|---|---|---|
| A. Editing and cursors | 32 | 4 | 11 | 17 |
| B. View rendering | 8 | 2 | 3 | 3 |
| C. Widgets and menus | 53 | 6 | 20 | 27 |
| D. Navigation, folding, scrolling and sticky scroll | 35 | 6 | 8 | 21 |
| E. Diff editor and multi diff editor | 24 | 0 | 7 | 17 |
| F. Options, themes and languages | 13 | 2 | 4 | 7 |
| G. Public API | 24 | 1 | 6 | 17 |
| H. Input, accessibility and keybindings | 15 | 1 | 6 | 8 |
| **Total** | 204 | 22 | 65 | 117 |

Merged across audits (each listed once): column-selection paste (§A.17), Go to Bracket (§A.27), the content hover's dismissal (§C.1), gutter hovers (§C.8), unfolding on cursor moves (§D.1), revealing all cursors (§D.18), PageUp/PageDown (§D.19), plain text (§F.2), `word_at_position` (§G.21) and the screen reader announcements (§H.13, §H.14).

## A. Editing and cursors

Scope: typing, cursor movement and multi-cursor, clipboard, formatting and snippet edits, line and bracket commands. The text model (undo grouping, EOL, BOM, `pushEditOperations`, search) had no findings except §G.21.

Checked and matching: the `typeWithInterceptors` order, auto-close and overtype, surround and electric characters, `getEnterAction` and indent-on-type, delete-pair, the undo-stop rules, Left/Right collapse, the leftover visible column on Up/Down, the Home toggle, atomic soft tabs on arrows, every `WordNavigationType`, deleteWord heuristics, word parts, cursor merging and cursor undo, moveLines with `autoIndent: full`, copy/delete/join/insert lines, transpose, case transforms, line and block comments, reindent, auto-indent on paste, the indentation guesser and conversions, smart select, EOL handling, `setEOL`/`pushEOL`, BOM, alternative version ids, trim-auto-whitespace and `textModelSearch`.

### Typing and language rules (#331)

#### A.1 Indent and outdent break doc-comment continuation lines (P1)
- **Monaco:** `common/commands/shiftCommand.ts:121-176`. With `useTabStops`, a line whose content starts off an indent stop runs `getEnterAction` on the line above, and the leading spaces of `appendText` (` * `), minus `removeText`, count as alignment. This applies to Tab and Shift+Tab with a selection and to Indent/Outdent Lines.
- **Port:** `ShiftCommand` has no language input ("without the language-dependent alignment fix-up", `cursor/typing.rs:313`, logic `:374-425`; module header `:18-19`).
- **Repro:** TypeScript, 4 spaces, text `/**`, ` * a`, ` */`, `x();`. Select all and press Tab. Monaco: `    /**`, `     * a`, `     */`. Port: `    * a`, `    */` (the alignment space is eaten). Shift+Tab on the unindented text changes nothing in Monaco; the port gives `* a` and `*/`.
- **Tests:** `shiftCommand.test.ts` #348 (`test/browser/commands/shiftCommand.test.ts:567`), #1609 (`:635`) and #1620 a–d aren't ported.

#### A.2 IME and dead-key commits skip auto-close, overtype and surround (P2)
Not the tracked IME clause-underline item (#305).
- **Monaco:** `endComposition` runs `TypeOperations.compositionEndWithInterceptors` (`common/cursor/cursor.ts:549-561`, `common/cursor/cursorTypeOperations.ts:73-157`). While composing, `type` passes `isDoingComposition` (`cursor.ts:578`), so interceptors are skipped.
- **Port:** the marked text is committed with a plain replace (`editor.rs:6036-6046`), and the `type` command runs interceptors between `compositionStart` and `compositionEnd` (`editor_commands.rs:206`). Header: `cursor/language_typing.rs:19-21`.
- **Repro:** macOS US-International. Type `'` then Space on an empty JS line: Monaco `''`, port `'`. Select `test`, type `'` then Space: Monaco `'test'`, port `'`.
- **Tests:** the IME cases in `test/browser/controller/cursor.test.ts:6055-6296` (#15825, #90016, #2773, #26820, #144690, #144693, #82701).

#### A.3 Closers inserted by snippets aren't tracked as auto-closed (P2)
- **Monaco:** `executeEdits` with source `'snippet'` records auto-closed pairs in the inserted text (`common/cursor/cursor.ts:439-536`); the snippet session uses that source (`contrib/snippet/browser/snippetSession.ts:709,735`).
- **Port:** snippet edits pass `auto_closed: Vec::new()` (`snippet_session.rs:354,727`; header `cursor/language_typing.rs:21`).
- **Repro:** Monaco test #78975 (`cursor.test.ts:6006`). Accept a completion whose snippet is `id="$1"`, type `a`, then `"`. Monaco `id="a"`, port `id="a""`. `log($1)` then `)` is the same.

#### A.4 Insert Line Above/Below ignore onEnter and indentation rules (P2)
- **Monaco:** `EnterOperation.lineInsertBefore/After` call `_enter` (`common/cursor/cursorTypeEditOperations.ts:661-695`, from `contrib/linesOperations/browser/linesOperations.ts:679,705`).
- **Port:** `insert_line_above/below` (`cursor/lines.rs:700-758`) use `typing::enter_command` (`cursor/typing.rs:184`), which copies the indentation only.
- **Repro:** JS, cursor on `if (x) {`, Cmd+Enter. Monaco indents the new line one level deeper; the port keeps the `if` line's indentation.

#### A.5 Edits from outside the editor drop auto-closed tracking (P3)
- **Monaco:** auto-closed ranges are model decorations and survive undo and other editors' edits (`common/cursor/cursor.ts` `_validateAutoClosedActions`).
- **Port:** any edit not made through the editor's `apply` clears them (`editor.rs:2447-2449`): undo/redo, `model.apply_edits`, a second editor.
- **Repro:** `foo(|)`; a second editor on the model inserts on another line; type `)`. Monaco `foo()`, port `foo())`.

#### A.6 Multi-character `type` runs no interceptors (P3)
- **Monaco:** `type(text, 'keyboard')` runs `typeWithInterceptors` per character (`common/cursor/cursor.ts:564-585`).
- **Port:** text longer than one grapheme goes to `type_text` (`cursor/language_typing.rs:81-83`). This contradicts `book/src/model-and-view.md:96` ("like typing: auto-close, auto-indent").
- **Repro:** `trigger("keyboard", "type", {text: "f("})`. Monaco `f()`, port `f(`.

#### A.7 Stale "not ported" comments about overtype mode (P3)
- `cursor/typing.rs:18` and `cursor/language_typing.rs:20` say overtype (Insert) mode isn't ported. It is: `overtype_cursor_style`/`overtype_on_paste` (`options.rs:2067-2069`), `cursor/tests/overtype_test.rs`, and `MONACO_PARITY.md` marks it ✅.

### Cursor movement and multi-cursor (#333, #334)

#### A.8 Word moves run on wrapped segments instead of model lines (P1)
- **Monaco:** word moves work on model positions with `model.getLineContent`, so wrapping has no effect (`contrib/wordOperations/browser/wordOperations.ts:44-62`).
- **Port:** every move other than Left/Right/Home/End/LineStart/LineEnd takes the `_` branch at `editor.rs:3232-3241`, which runs `cursor::move_cursors` on the view model. `ViewLines::line_content` returns the segment's text with the wrap indent as spaces (`view/view_lines.rs:452-462`).
- **Repro:** word wrap on, `wrappingIndent: same`, an indented line `    foo(alpha, beta, gamma, delta)` that wraps. Caret at the start of the second visual line, Alt+Left. Monaco moves to the previous word's start; the port only moves into the wrap indent (the model position doesn't change, and Shift+Alt+Left selects nothing). A word split by a wrap also stops at the wrap point.

#### A.9 With the default `selectedTextMatchMode: findOptions`, Cmd+D and selection highlights are case-sensitive (P1)
- **Monaco:** `getSelectionSearchOptions` reads the find state's `matchCase` and `wholeWord` whenever the mode is `findOptions`, even with find closed; both default to false (`contrib/multicursor/browser/multicursor.ts:284-291`; test "default selected-text matching uses Find match case and whole word options", `contrib/multicursor/test/browser/multicursor.test.ts:601`).
- **Port:** `selected_text_match_options` returns case-sensitive unless `self.find.visible` (`multi_cursor.rs:85-96`); selection highlights use it too (`occurrences.rs:470`). `book/src/options.md:126` documents this and `tests/multicursor.rs:293,321` bake it in, though their Monaco originals set `selectedTextMatchMode: 'caseSensitive'`.
- **Repro:** lines `foo`, `FOObar`, `foobar`. Select `foo` on line 1, Cmd+D. Monaco adds the `FOO` on line 2; the port adds `foo` on line 3.

#### A.10 Column selection works on model lines, so it lands inside folds and ignores wrapping (P2)
- **Monaco:** `ColumnSelection.columnSelect` works on view positions: folded lines are skipped and each wrapped segment gets a cursor.
- **Port:** the keyboard path (`editor.rs:3768-3775`) and the mouse path (`editor.rs:4946-4955`) run `cursor/column.rs` on the model rope.
- **Repro:** fold lines 2–5 under line 1, Cmd+Shift+Alt+Down on line 1. Monaco puts the new cursor on line 6; the port puts it on hidden line 2, and typing edits inside the fold.

#### A.11 Add Cursor Above/Below and keyboard column select don't push an undo stop (P2)
- **Monaco:** both call `model.pushStackElement()` (`contrib/multicursor/browser/multicursor.ts:86,138`; `browser/coreCommands.ts:421`).
- **Port:** neither pushes a stack element nor resets `prev_edit_kind` (`editor.rs:3291-3307`, `:3768-3775`), unlike `move_cursors` (`:3259-3262`).
- **Repro:** type `foo`, Cmd+Alt+Down, type `x`, Undo. Monaco removes only the two `x`s; the port also removes `foo`.

#### A.12 Edits from outside the editor leave a collapsed cursor before the inserted text (P2)
- **Monaco:** cursors are tracked ranges with `AlwaysGrowsWhenTypingAtEdges`; an empty selection is recovered at the range's end (`common/cursor/oneCursor.ts:50-70`).
- **Port:** foreign edits transform anchor and head with `Bias::Left` (`editor.rs:2429-2445`).
- **Repro:** `sideEditing.test.ts:78`: cursor at (1,1), `model.apply_edits` inserts `"something "` there. Monaco moves the cursor to (1,11); the port leaves it at (1,1). Monaco #111513 (`cursor.test.ts:6436`), two editors on one model: typing in one moves the other's cursor in Monaco only.
- **Tests:** `test/browser/commands/sideEditing.test.ts` isn't ported.

#### A.13 `stickyTabStops` doesn't snap mouse clicks (P2)
- **Monaco:** mouse targets in soft-tab indentation snap to the nearest tab stop (`browser/controller/mouseTarget.ts:551-555,1085-1092`, `Direction.Nearest`).
- **Port:** `AtomicDirection::Nearest` exists (`cursor/strings.rs:416`) but nothing calls it; only the arrow keys read `sticky_tab_stops`.
- **Repro:** `stickyTabStops` on, 8-space indent, click at column 1. Monaco puts the caret at column 0; the port at column 1.

#### A.14 Cmd+D with several empty cursors starts a whole-word, case-sensitive session (P3)
- **Monaco:** with several selections and any empty, Cmd+D only expands each to its word and starts no session (`contrib/multicursor/browser/multicursor.ts:592-608`).
- **Port:** `from_word = selections.len() == 1 || all empty` starts a session (`multi_cursor.rs:221`).
- **Repro:** lines `foo`, `foo`, `Foo`; collapsed cursors on lines 1 and 2; Cmd+D twice. Monaco adds `Foo`; the port adds nothing.

#### A.15 `cursorMove` viewport targets count partly visible lines (P3)
- **Monaco:** `viewPortTop`/`Bottom`/`Center` use `getCompletelyVisibleViewRange()` (`common/cursor/cursorMoveCommands.ts:358`).
- **Port:** `visible_rows` comes from `layout.lines`, partly visible lines included (`editor_commands.rs:533-547`, `api.rs:969-985`).
- **Repro:** with the last line half visible, run `cursorMove {to: "viewPortBottom"}`. The port puts the cursor on the half-visible line.

#### A.16 Read-only multi-cursor commands and unreachable arguments (P3)
- **Read-only:** Monaco makes Add Cursor Above/Below and Focus Next/Previous Cursor no-ops in read-only editors (`contrib/multicursor/browser/multicursor.ts:82,134,1144,1182`); the port runs them.
- **Arguments:** `deleteInsideWord {onlyWord}` and `insertCursorAbove/Below {logicalLine: false}` can't be passed, because those actions are unit structs (`editor.rs:6414-6416`).
- **Repro:** in a read-only editor, press Cmd+Alt+Down: the port adds a cursor.

### Clipboard, formatting and snippet edits (#351)

#### A.17 Column-selection ("block") paste is missing (P2)
Also found by the input audit (I2).
- **Monaco:** copying a real column selection records `isBlock` (`viewModel.getCursorColumnSelectData().isReal`, set by the column select commands at `browser/coreCommands.ts:429`; `browser/controller/editContext/clipboardUtils.ts:38-48`). Pasting a block into one cursor with `multiCursorPaste: spread` runs `ColumnSelectionPasteCommand` (`common/cursor/cursorTypeEditOperations.ts:704-726`, `common/commands/columnSelectionPasteCommand.ts`), which puts each row on the next line at the same visible column, pads short lines and appends lines past the end.
- **Port:** `ClipboardMetadata` has no block flag (`editor.rs:313-327`, written at `:3878-3887`); `cursor::paste` has no block branch (`cursor/typing.rs:1015-1080`); nothing ports the command.
- **Repro:** three lines `left--right`. Column-select `A`/`BC`/`D`, copy, put one cursor at line 1 column 5, paste. Monaco: `leftA --right`, `leftBC--right`, `leftD --right`. The port inserts `A\nBC\nD` as text.
- **Tests:** `test/browser/controller/columnSelectionPaste.test.ts` (about 50 cases).

#### A.18 Copy always uses `\n` (P3)
- **Monaco:** `getPlainTextToCopy` uses the model's EOL (or CRLF on Windows) for multi-line ranges, whole-line copies and the join between cursors (`common/viewModel/viewModelImpl.ts:1005-1055`, `browser/controller/editContext/clipboardUtils.ts:52`).
- **Port:** `copy_text` hard-codes `\n` (`cursor/typing.rs:990,1011`); `write_selection_to_clipboard` writes it unchanged (`editor.rs:3853-3880`).
- **Repro:** copy two lines of a CRLF model and paste into another app. Monaco puts `\r\n` on the clipboard; the port `\n`.

#### A.19 Format edits and workspace edits don't force-move markers (P3)
- **Monaco:** `FormattingEdit.execute` uses `EditOperation.replaceMove` except for a single full-model replace (`contrib/format/browser/formattingEdit.ts:53-58`); the standalone bulk edit service does the same (`standalone/browser/standaloneServices.ts:858`).
- **Port:** format edits (`format.rs:493`) and workspace edits (`bulk_edit.rs:318`) go through `execute_edits`, which uses `force_move_markers: false` (`api.rs:612-617`).
- **Repro:** line `\tfoo`, cursor at column 0, a formatter turning the tab into 4 spaces. Monaco ends with the cursor before `foo`; the port leaves it at 0. Rename (F2) with the cursor before the identifier shows the same.

#### A.20 Format on paste runs with several cursors (P3)
- **Monaco:** `FormatOnPaste._trigger` returns with more than one selection (`contrib/format/browser/formatActions.ts:205`).
- **Port:** `format_on_paste` always formats (`format.rs:587`), over a range from the first cursor to the last (`editor.rs:3974`).
- **Repro:** `format_on_paste` on, two cursors on different lines, paste. Monaco formats nothing; the port formats everything between them.

#### A.21 Formatters can't change the line ending (P3)
- **Monaco:** `TextEdit.eol` is applied with `pushEOL` (`contrib/format/browser/formattingEdit.ts:15-34`).
- **Port:** a `TextEdit` is `(Range<PointUtf16>, String)` (`providers.rs:2384`).
- **Repro:** a formatter returning an edit with `eol: CRLF` has no way to say so.

#### A.22 Snippet `overwriteBefore`/`overwriteAfter` with a selection replace the whole selection (P3)
- **Monaco:** `snippetSelection = selection.setStartPosition(extensionBefore.start).setEndPosition(extensionAfter.end)`, measured from the head (`contrib/snippet/browser/snippetSession.ts:540-552`).
- **Port:** `before.start().min(selection.start())..after.end().max(selection.end())` (`snippet_session.rs:450`).
- **Repro:** select all of `console.log` left to right, `insert_snippet("far", overwrite_before: 3)`. Monaco `console.far`; port `far`.

#### A.23 Snippet choices aren't offered again after the list is dismissed (P3)
- **Monaco:** while a choice is active, the controller registers an exclusive completion provider (`contrib/snippet/browser/snippetController2.ts:163-225`).
- **Port:** choices are shown once (`snippet_session.rs:1111-1190`); `request_suggestions_with` doesn't check for an active choice (`completion.rs:699`).
- **Repro:** insert `${1|one,two,three|}`, Escape, Ctrl+Space. Monaco lists one/two/three; the port asks the normal providers.

#### A.24 Snippet path variables for non-`file://` URIs (P3)
- **Monaco:** standalone uses `resource.path`: for `http://h/abc/def/ghi`, `TM_FILEPATH` is `/abc/def/ghi` and `TM_DIRECTORY` is `/abc/def` (`contrib/snippet/test/browser/snippetVariables.test.ts:83-92`).
- **Port:** `TM_FILEPATH` and `RELATIVE_FILEPATH` return the whole URI; `TM_DIRECTORY` and `TM_DIRECTORY_BASE` return `None` (`snippet/variables.rs:238-275`).
- **Repro:** insert `$TM_DIRECTORY` in a model with uri `http://h/abc/def/ghi`.

### Line and bracket commands (#328)

#### A.25 In-place replace goes the wrong way (P1)
- **Monaco:** `inPlaceReplace.up` (Replace with Previous Value, Cmd/Ctrl+Shift+,) calls `run(id, false)` and `.down` (Cmd/Ctrl+Shift+.) calls `run(id, true)` (`contrib/inPlaceReplace/browser/inPlaceReplace.ts:129-174`); `up = true` increments (`common/languages/supports/inplaceReplaceSupport.ts:46-98`).
- **Port:** `InPlaceReplaceUp` calls `in_place_replace(true)` and `Down` calls `false` (`editor.rs:6480-6485`), with Monaco's keys (`actions.rs:1727-1728`). `tests/editor.rs:736-747` expects `1` → `2` on Cmd+Shift+,.
- **Repro:** `foo(1)`, cursor on `1`, Cmd+Shift+,. Monaco `foo(0)`; port `foo(2)`.

#### A.26 Trim Trailing Whitespace also trims inside strings and regexes (P2)
- **Monaco:** the action reads `files.trimTrailingWhitespaceInRegexAndStrings`, which standalone never sets (`contrib/linesOperations/browser/linesOperations.ts:486-488`), so it skips lines whose trailing whitespace starts in a String or RegEx token, or whose tokens aren't accurate yet (`common/commands/trimTrailingWhitespaceCommand.ts:101-115`).
- **Port:** trims every line (`cursor/transforms.rs:294-320`).
- **Repro:** a JS template literal whose first line ends in spaces, Cmd+K Cmd+X. Monaco keeps them; the port deletes them.
- **Tests:** "skips strings and regex if configured" (`test/browser/commands/trimTrailingWhitespaceCommand.test.ts:120`).

#### A.27 Go to Bracket and Select to Bracket lack fallbacks, direction and arguments (P2)
Also found by the navigation audit (4.2).
- **Monaco** (`contrib/bracketMatching/browser/bracketMatching.ts`): with no matching or enclosing pair, Jump goes to the next bracket and Select selects the next bracket's pair (`findNextBracket`, `:182-272`, `:200-211`, `:236-243`); Select runs backwards from a closing bracket (`:255-260`); `selectBrackets: false` selects only the contents (`:56-77`).
- **Port** (`bracket_matching.rs:154-192`): both stay put with no pair, Select always runs forwards, and `SelectToBracket` is a unit struct.
- **Repro:** `var x = (3 + (5-7)); y();`, cursor at 1-based column 21, Go to Bracket. Monaco jumps to column 23; the port doesn't move.
- **Tests:** none of the 7 in `contrib/bracketMatching/test/browser/bracketMatching.test.ts`.

#### A.28 The tab-size pickers start on "1", not the current size (P2)
- **Monaco:** `activeItem: picks[min(tabSize - 1, 7)]` (`contrib/indentation/browser/indentation.ts:134-139`).
- **Port:** `show_quick_pick` has no active item (`indentation_commands.rs:128`).
- **Repro:** tab size 4, "Indent Using Spaces", Enter. Monaco keeps 4; the port sets 1. Same for "Indent Using Tabs" and "Change Tab Display Size".

#### A.29 Sort Lines doesn't follow `Intl.Collator` (P3)
- **Monaco:** the default `Intl.Collator` (`contrib/linesOperations/browser/sortLinesCommand.ts:16,89`): punctuation, symbols, digits, letters, accents beside their base.
- **Port:** lowercased code points (`cursor/transforms.rs:90-105`).
- **Repro:** sort `b`, `_a`, `1`, `}`, `é`, `f`, `e`. Monaco `_a } 1 b e é f`; port `1 _a b e f } é`.

#### A.30 The in-place replace flash shows in every theme (P3)
- **Monaco:** `.valueSetReplacement` is styled only under `.monaco-editor.vs`, as an `editorBracketMatch.border` outline (`contrib/inPlaceReplace/browser/inPlaceReplace.css`).
- **Port:** an `inputOption.activeBorder` border in every theme (`editor.rs:3539-3542`).
- **Repro:** run Replace with Next Value in `vs-dark`: the port flashes a border, Monaco doesn't.

#### A.31 Bracket highlighting stops at 32 cursors instead of 100 (P3)
- **Monaco:** skips bracket matching above 100 selections (`contrib/bracketMatching/browser/bracketMatching.ts:345-346`).
- **Port:** `MAX_CURSORS = 32` (`bracket_matching.rs:17`).
- **Repro:** put 50 cursors next to brackets: Monaco highlights the matches, the port doesn't.

### Test coverage (#357)

#### A.32 Monaco editing test suites not ported (P3)
The suites tied to a finding are listed with it (A.1, A.2, A.12, A.17, A.26, A.27, A.9). These have no single fix:
- `test/browser/controller/cursorMoveCommand.test.ts`: 55 tests; the port has a smoke test (`tests/commands.rs:485`). The viewport (A.15), halfLine and foldedLine suites are missing.
- `contrib/linkedEditing/test/browser/linkedEditing.test.ts`: about 35 cases against 5 (`tests/linked_editing.rs`). Missing: boundary inserts and "Selection insert - across boundary", the paste and stop-pattern breakouts, deletes with undo, multi-line inserts.
- `contrib/snippet/test/browser/snippetVariables.test.ts`: 17 tests against 6. "editor variables, file/dir" would catch A.24.
- No integration test types into an editor without a language (§F.2); the unit harness supplies its own plaintext config (`cursor/tests/language_configs.rs:100-115`).
- Neither codebase tests word moves under wrap (A.8), reveal after Cmd+D, Add Cursor or column select (§D.18), PageDown's scroll position (§D.19), or column select across folds (A.10).

## B. View rendering

Scope: `element.rs`, `view/*`, `decorations.rs`, `theme.rs`, `bracket_colors.rs`, `unicode_highlight*` and `layout_info*`, against `browser/viewParts/*`, `common/viewLayout/viewLineRenderer.ts`, `common/viewLayout/viewLayout.ts` and the contrib decorations. B.1, B.4 and B.5 were confirmed in playground screenshots (release build, `--features screenshot`, `PLAYGROUND_PLAIN=1`).

Checked and matching (audit 2 §E and the gutter items of §C): paint order, selection corners and end-of-line width, the current-line highlight, the block cursor and cursor geometry, multi-cursor colors, the whitespace marks, guide colors and `_maxIndentLeft`, control characters, gutter widths, line number modes, rulers, the wrap indicator, the overflow chip, the fold placeholder and fold-control fade.

### Text layout (#327)

#### B.1 Tabs after full-width characters or emoji land one column too far (P1)
- **Monaco** (`common/viewLayout/viewLineRenderer.ts:1150-1218`): a full-width character adds 2 to `visibleColumn` (`:1195-1197`), each UTF-16 unit adds 1 (an astral emoji counts 2), and a tab produces `tabSize - visibleColumn % tabSize` spaces (`:1154-1156`).
- **Port** (`element.rs:884-907`, `layout_segment`): `visible_column += 1` per `char`, so the tab gets one space too many. This disagrees with the port's own wrapping (`view/line_breaks.rs:138-146` counts full-width as 2).
- **Repro:** `abcd\tX`, `中\tX`, `ab\tX`, `😀\tX`, tab size 4, whitespace on. `X` after `中` sits at column 5, not 4; the emoji line is off too.
- **Fix:** count 2 for `is_full_width_character`, `len_utf16()` otherwise.

#### B.2 Word wrap still allows a few columns of horizontal scrolling (P1)
- **Monaco** (`common/viewLayout/viewLayout.ts:313-333`, `_computeContentWidth`): when wrapping, the scroll width is `maxLineWidth`; `scrollBeyondLastColumn` and the scrollbar width are added only when not wrapping. The horizontal reveal skips `_ensureMaxLineWidth` while wrapping (`browser/viewParts/viewLines/viewLines.ts:639`).
- **Port** (`element.rs:3348-3350`): `content_width.max(line.width() + em_width * scroll_beyond_last_column)` runs either way, so `max_scroll_left > 0` (`:3355`). The horizontal autoscroll (`:3360-3372`) and sideways wheel then scroll wrapped text, with no scrollbar (`show_hbar` was decided at `:2942`).
- **Repro:** `PLAYGROUND_WRAP=on PLAYGROUND_WIDTH=700`, one 300-character line of `a`, press End. Every view line shifts left and the first `a` is clipped at the gutter.

#### B.3 Indent guides are drawn through wrapped continuation lines with `wrapping_indent: None` (P2)
- **Monaco** (`common/viewModel/viewModelLines.ts:692-697,724-737`): continuation lines that start at column 1 get `IndentGuideRepeatOption.BlockSubsequent` (indent 0), or `BlockAll` when the visible range starts mid-line.
- **Port** (`element.rs:4632-4713`): `count = counts[line.row - rows.start]` for every view line of a row.
- **Repro:** `word_wrap: On, wrapping_indent: None`, a two-level indented line that wraps. The guides at columns 0 and 4 cross the continuation text.

### Decorations overlay (#345)

#### B.4 Unicode highlights don't widen invisible or narrow characters (P2)
- **Monaco:** the decoration is `className: 'unicode-highlight', showIfCollapsed: true` (`contrib/unicodeHighlighter/browser/unicodeHighlighter.ts:557-566`); `_renderNormalDecoration` widens ranges narrower than `typicalHalfwidthCharacterWidth` to a centered half-width box (`browser/viewParts/decorations/decorations.ts:196-204`).
- **Port:** `show_if_collapsed` is false (`unicode_highlighter.rs:217-224`), so `widen_collapsed` (`element.rs:4028-4030`) never runs. A zero-width space is a 2px sliver (`element.rs:4033-4036`); a thin space a narrow box.
- **Repro:** `a​b` (zero-width space) and `c d` (thin space).
- **Fix:** set `show_if_collapsed: true`.

#### B.5 Find-match decorations lack `showIfCollapsed` and the end-of-line extension (P2)
- **Monaco:** all three find decorations have `showIfCollapsed: true` (`contrib/find/browser/findDecorations.ts:286-325`): empty regex matches show a character-wide box, and a match ending at column 1 of the next line is clipped to the previous line (`browser/viewParts/decorations/decorations.ts:135-138`). `findMatch` renders with `includeNewLines` (`decorations.ts:187`), so lines a multi-line match continues past get the half-width EOL extension.
- **Port:** `find.rs:270-312` (`match_options`, `current_match_options`) set no `show_if_collapsed`; `decoration_x_range` (`element.rs:6572-6603`) adds no EOL width. Minor: the no-overview variant has `z_index: 10`, Monaco's 0.
- **Repro:** regex search `^`: only 2px bars at line starts. Search `;\n`: the match stops at the text end, with a sliver on the next line.

#### B.6 Every empty decoration piece gets a 2px minimum width (P3)
- **Monaco:** a zero-width `className` piece renders as a 0px div (`browser/viewParts/decorations/decorations.ts:206-222`); the only 2px minimum is the snippet placeholder's CSS (`contrib/snippet/browser/snippetSession.css:7`).
- **Port:** `(to - from).max(px(2.))` for every decoration with a background or border (`element.rs:4033-4036`).
- **Repro:** a decoration collapsed by deleting its text, an empty line inside a multi-line background decoration, or the column-0 end of `1:1-3:1` each show a 2px bar.

#### B.7 Whole-line decorations aren't drawn under regular ones (P3)
- **Monaco:** `DecorationsOverlay.prepareRender` renders `isWholeLine` decorations first, then the others; zIndex orders within each pass (`browser/viewParts/decorations/decorations.ts:108-110`).
- **Port:** sorts by `(z_index, start)` only (`decorations.rs:695`), painted in that order (`element.rs:3991-4039`).
- **Repro:** a whole-line decoration with a higher `z_index` (a debugger line highlight) covers find matches and word highlights.

#### B.8 The fold background is painted under selections and guides (P3)
- **Monaco:** `foldingHighlight` is a whole-line `folded-background` decoration in `DecorationsOverlay`; overlays stack current line < selections < indent guides < decorations (`browser/view.ts:220-224`).
- **Port:** `fold_highlights` is painted after the current-line highlight, before `paint_selections` and guides (`element.rs:5289-5292`, `:5355-5366`).
- **Repro:** select text on a folded line: the selection covers the translucent `editor.foldBackground` tint instead of being tinted, and guides draw over it.

## C. Widgets and menus

Scope: the hover and marker zone, suggest and parameter hints, code actions and the lightbulb, rename, the context menu, the color picker, find, quick input, the inline suggestion toolbar and inline-edit menus, peek and the paste/drop widgets. The audit 2 items re-checked here (§A 1–4, 9, 11, 12, 19, 20, 22–24; §B 2, 6–13, 15–20, 22, 24) are fixed, except for the two partial gaps in C.3 and C.47.

### Content hover (#335)

#### C.1 The content hover stays up after keys, clicks, drags and the mouse leaving, and hides on wheels that don't scroll (P1)
Also found by the input audit (I1 and M3).
- **Monaco** (`contrib/hover/browser/contentHoverController.ts`): hides on a scroll that changes the position (`:111`, `:123-136`), a mouse down that isn't on the hover (`:132-145`), the mouse leaving the editor unless the hover is sticky and the mouse is over it, which also cancels a pending hover (`:162-177`), the start of a drag-select, which also suppresses new hovers (`:213-216`), and any key that isn't a modifier, a chord prefix or a hover command (`:268-311`). The key isn't consumed, so its command still runs.
- **Port:** `hide_hover` runs only on focus-out (`editor.rs:771`), model change (`:1012`), edits (`:2561`), Escape when nothing ranks higher (`:3374-3377`) and every wheel event (`scroll_by_wheel`, `:5821`), even when nothing scrolls or `scrollbar.handle_mouse_wheel` is off. The mouse-leave handler (`:7213-7227`), the keystroke interceptor (`:784-799`) and `on_key_down` (`:7204-7206`) never hide it, and hover logic is skipped while a button is down (`:5272`).
- **Repro:** hover a word until the card shows, then press ← or click the word: the card stays. Rest on a word and leave the editor within 300ms: the card appears and never goes away. Cmd/Ctrl+K Cmd/Ctrl+I, then arrow keys: the hover stays while the cursor leaves. With several cursors, Escape takes two presses. At the top of a file, wheel up: the hover closes though nothing scrolled.

#### C.2 Moving along a long squiggle makes the hover blink (P2)
- **Monaco:** a new anchor on the same line adopts the visible hover and keeps the parts that are still valid (`contrib/hover/browser/contentHoverWidgetWrapper.ts:130-147`, `canAdoptVisibleHover`, `markerHoverParticipant.ts:44-50`); the card is anchored at the parts' range start.
- **Port:** the hover is kept only while the mouse is inside the word or provider range (`hover.rs:1816-1827`); otherwise it hides and waits the delay again (`:1871-1884`).
- **Repro:** a marker over `foo.bar(baz)`; hover `foo`, move to `baz`. The card vanishes for about 300ms and returns at `baz`.

#### C.3 Hovers and the marker zone have no scrollbar (P3)
Left over from audit 2 §A.
- **Monaco:** the marker zone has a `ScrollableElement` with 6px scrollbars (`contrib/gotoError/browser/gotoErrorWidget.ts:76-82`); the hover's is upstream, not vendored.
- **Port:** plain `overflow_scroll` (`hover.rs:500-516`, `markers.rs`). The wheel works, but nothing shows that it scrolls.
- **Repro:** open a hover taller than its maximum height.

#### C.4 The unicode hover's order and layout (P3)
- **Monaco:** ordinals are marker 1, color 2, markdown 3, inline completions 4, unicode 5 (`contrib/unicodeHighlighter/browser/unicodeHighlighter.ts:417`); "Adjust settings" is an inline link after the reason (`:503-507`).
- **Port:** unicode parts come first (`hover.rs:289-290`); "Adjust settings" is its own row and the reason sits in a fixed 480px box (`hover.rs:657-686`).
- **Repro:** hover an ambiguous character that also has a marker or provider hover.

#### C.5 Unicode banner and related-information link details (P3)
- **Banner:** Monaco's is focusable with `aria-label` set to its message (`contrib/unicodeHighlighter/browser/bannerController.ts:59, 89-93`); `UnicodeBanner::render` has neither.
- **Related information:** Monaco colors the link `textLink.activeForeground` and closes the zone on click (`contrib/gotoError/browser/gotoError.ts:107-113`); the port uses `text_link_foreground` and stays open.
- **Repro:** tab to the banner with a screen reader on, or click a related-information file name in the marker zone.

### Gutter and decoration hovers (#329)

#### C.6 A whole-line decoration's `hover_message` only shows inside its range (P1)
From the API audit (B1).
- **Monaco:** `ContentHoverComputer._getLineDecorations` keeps every `isWholeLine` decoration on the hovered line (`contrib/hover/browser/contentHoverComputer.ts:44-47`) and widens `showIfCollapsed` ones by a character each side (`:52-56`); the range is clamped to the hovered line (`markdownHoverParticipant.ts:147-161`). `{ range: (5,1,5,1), options: { isWholeLine: true, hoverMessage } }`, a common blame pattern, hovers anywhere on line 5.
- **Port:** keeps only decorations with `start <= offset <= end` (`hover.rs:2088-2103`); no `is_whole_line` or `show_if_collapsed` handling, and a multi-line decoration's whole range is the hover range.
- **Repro:** the example above hovers only at column 0 in the port.

#### C.7 The gutter hover is placed like a content hover (P2)
- **Monaco:** `GlyphHoverWidget._showAt` puts the card just right of the glyph margin or line numbers, centred vertically on the line (`contrib/hover/browser/glyphHoverWidget.ts:167-196`).
- **Port:** `show_hover(start..start)` at column 0, so the card goes above or below and covers other lines (`hover.rs:1950-1966, 2256-2271`).
- **Repro:** give a decoration a `glyph_margin_hover_message` and hover its glyph.

#### C.8 Gutter hovers: only decorations starting on the row, no lane filter, no `lineNumberHoverMessage` (P2)
Found by the widgets audit (A8) and the API audit (B4).
- **Monaco:** `GlyphHoverComputer.computeSync` takes `getLineDecorations(lineNumber)`, every decoration intersecting the line (`contrib/hover/browser/glyphHoverComputer.ts:36-38`); in the glyph margin it keeps only the hovered lane's (`:47-50`, `glyphHoverWidget.ts:106-116`); over line numbers it shows `lineNumberHoverMessage` (`:52`).
- **Port:** the glyph margin and line numbers are one `HoverTarget::Gutter(row)` (`editor.rs:5325-5337`). `glyph_margin_hover_messages` keeps decorations whose start row is the row (`hover.rs:1968-1994`), ignores lanes, and shows for line numbers too; `DecorationOptions` has no `line_number_hover_message` (`decorations.rs:353-356`).
- **Repro:** hover the line number of a row with a glyph hover: the port shows the glyph's message. Hover line 2 of a three-line decoration's glyph: nothing.

#### C.9 Decoration hover messages are one plain string (P2)
From the API audit (B5).
- **Monaco:** `hoverMessage`, `glyphMarginHoverMessage` and `lineNumberHoverMessage` are `IMarkdownString | IMarkdownString[]` (`monaco.d.ts:1709-1717`) with `isTrusted`, `supportHtml` and `supportThemeIcons`.
- **Port:** `Option<SharedString>` (`decorations.rs:356`, `388`), wrapped in an untrusted `MarkdownString::new` (`hover.rs:2099`, `1987`).
- **Repro:** a decoration hover with a `command:` link, a `$(icon)` or HTML doesn't run or render, and only one message can be given.
- **Fix:** use `Vec<MarkdownString>` (the type exists in `markdown.rs`).

### Marker navigation (F8) (#330)

#### C.10 F8 visits Hint markers and counts them as problems (P1)
- **Monaco:** `MarkerList` reads Error, Warning and Info only (`contrib/gotoError/browser/markerNavigationService.ts:64-67`).
- **Port:** `go_to_marker` and `show_marker` use all of `model.markers()` (`markers.rs:734-765`).
- **Repro:** the playground's demo markers (`providers.rs:825-866`), cursor on line 1, Alt+F8. The zone opens on the "unused import" hint, "1 of 4 problems". Monaco shows the line 11 error, "1 of 2".

#### C.11 F8 order and starting marker differ (P2)
- **Monaco:** sorts by severity, then position (`markerNavigationService.ts:51-62`); the first move picks the marker containing the cursor (`:136-150`); later moves step an index, so markers with the same start are each visited.
- **Port:** sorts by position and picks the first with `range.start > head` (`markers.rs:741-751`).
- **Repro:** click inside the line 11 squiggle, Alt+F8: the port skips to the next marker. The second of two markers with the same start is never reached.

#### C.12 The marker zone doesn't follow marker changes and has no stale state (P2)
- **Monaco:** on a marker change the zone moves to the marker now at its position, or turns stale (`contrib/gotoError/browser/gotoError.ts:94-104`): 0.6 opacity and italic (`gotoErrorWidget.ts:381-389`, `.stale` in `media/gotoErrorWidget.css`).
- **Port:** the `MarkersChanged` handler (`editor.rs:2508-2511`) ignores the zone, which holds a cloned `Marker`.
- **Repro:** press F8, then clear the markers from the host. The zone keeps the old message.

#### C.13 The marker zone isn't scrolled into view (P2)
- **Monaco:** `ZoneWidget.show` reveals the zone (`contrib/zoneWidget/browser/zoneWidget.ts:463-476`) and `showAtMarker` calls `revealPositionInCenterIfOutsideViewport` (`gotoErrorWidget.ts:369`).
- **Port:** `show_marker_at` only sets the selection (`markers.rs`).
- **Repro:** scroll a marker to the last visible line, Alt+F8: the zone opens below the viewport.

### Suggest widget and parameter hints (#344)

#### C.14 The suggest list is anchored at the word start, not the cursor (P2)
- **Monaco:** `setPosition(this.editor.getPosition())` on show and every refilter (`contrib/suggest/browser/suggestWidget.ts:542, 552`).
- **Port:** anchored at `session.word_start` (`completion.rs:1243`); "Loading..." too (`:771`).
- **Repro:** type `foobar`, Ctrl+Space at the end. Monaco's list starts under the cursor and follows typing; the port's starts under `f` and stays.

#### C.15 Details vanish while an item resolves, with no "Loading..." (P2)
- **Monaco:** the panel keeps its content, shows "Loading..." after 250ms, then re-renders (`suggestWidget.ts:420-425`, `suggestWidgetDetails.ts:123-129`).
- **Port:** `details_shown` needs `can_expand` of the unresolved item (`suggest_widget.rs:1469`), so the panel is removed until resolve finishes.
- **Repro:** a provider whose docs come from a ~300ms resolve; expand details and arrow through. The panel flickers and the list shifts.

#### C.16 "Show More" in the status bar uses `canExpand`, not `suggestionCanResolve` (P3)
- **Monaco:** `!!resolveCompletionItem || !!documentation || detail !== label` (`contrib/suggest/browser/suggestController.ts:212, 1029-1035`).
- **Port:** shown only when `can_expand(item)` (`suggest_widget.rs:1512`).
- **Repro:** `suggest.show_status_bar` with word-based suggestions: Monaco shows "Show More (⌃Space)", the port nothing.

#### C.17 Toggle Details flips the flag when the item has nothing to show (P3)
- **Monaco:** opens details only when `canExpandCompletionItem(focused)` (`suggestWidget.ts:725-740`).
- **Port:** toggles `details_expanded` unconditionally (`completion.rs:1450-1457`).
- **Repro:** Ctrl+Space on an item without docs, then Down to one with docs. The port opens details; Monaco doesn't.

#### C.18 Clicks, commit characters and Shift+Enter keep the Tab-completion alternatives (P3)
- **Monaco:** only Tab and Enter pass `KeepAlternativeSuggestions` (`suggestController.ts:166, 169, 477, 888-890`).
- **Port:** `accept_suggestion_with` always stores alternatives with `tab_completion == On` (`completion.rs:2201-2216`).
- **Repro:** `tab_completion: On`, click a suggestion, press Tab: the port cycles to the next suggestion.

#### C.19 Parameter hints don't follow the caret between requests (P3)
- **Monaco:** positioned at the live cursor, laid out again on each selection change (`contrib/parameterHints/browser/parameterHintsWidget.ts:106-110, 166-174`).
- **Port:** stays at the request's cursor until a response (`signature_help.rs:470, 512`).
- **Repro:** type `foo(aaaa, bbbb` quickly: the widget lags, then jumps.

#### C.20 Suggest rows have no hover background (P3, upstream, not vendored)
- **Monaco:** `getListStyles` paints `list.hoverBackground` on unfocused rows.
- **Port:** `render_row` has no `.hover` (`suggest_widget.rs:1376-1395`), though the theme reads `list_hover_background` (`theme.rs:738`).
- **Repro:** move the mouse over unfocused rows: no tint.

#### C.21 Row and sash polish (P3)
- **Icon column:** Monaco is 16px plus a 4px margin (`suggest.css .suggest-icon`); the port is `.w(item_height)` (`suggest_widget.rs:1332`), so a large `suggest.line_height` pushes labels right.
- **Sash double-click** (upstream, not vendored): Monaco resets the size; the port handles only `on_drag` (`suggest_widget.rs:1786-1817`).
- **Detail equal to the label:** Monaco still shows it in details (`suggestWidgetDetails.ts:155-165`); the port filters it (`suggest_widget.rs:1604`).
- **Repro:** `suggest.line_height: 32`; double-click the sash; open details on an item whose detail equals its label.

### Code actions, lightbulb, rename, context menu and color picker (#325, #343)

#### C.22 F2 on a symbol scrolled out of view does nothing (P1)
- **Monaco:** `RenameWidget._show` calls `revealLineInCenterIfOutsideViewport` (`contrib/rename/browser/renameWidget.ts:583`).
- **Port:** `show_rename_input` (`rename.rs:663-744`) never reveals; layout then sees the anchor isn't laid out and cancels the rename (`element.rs:5185-5195`).
- **Repro:** caret on an identifier, wheel it off screen, F2. Nothing appears.

#### C.23 The code action menu opens invisible but focused when the cursor is off screen (P1)
- **Monaco:** `toCoords` calls `revealPosition(…, Immediate)` and `render()` before anchoring (`contrib/codeAction/browser/codeActionController.ts:407-422`).
- **Port:** `show_code_action_list` adds and focuses a content widget without revealing (`code_action.rs:1086-1159, 1303-1382`); widgets on lines that aren't laid out aren't placed (`element.rs:5161`).
- **Repro:** scroll a diagnostic off screen, Cmd+. Nothing shows, and Enter applies a fix you can't see.

#### C.24 The lightbulb on the last line is drawn over the previous line's code (P2)
- **Monaco:** for the last line `nextLineEmptyOrIndented` is false, so the bulb goes to the gutter (`contrib/codeAction/browser/lightBulbWidget.ts:356-371`).
- **Port:** the gutter check adds `row < last` (`code_action.rs:362`). The cursor-overlap check compares a 0-based column with Monaco's 1-based one (`code_action.rs:383` vs `lightBulbWidget.ts:394`).
- **Repro:** `let a = 1;` / `let b = 2;`, a fix on line 2, caret there. The bulb covers line 1's `l`.

#### C.25 `lightbulb: Off` still shows the bulb after Cmd+. (P2)
- **Monaco:** `LightBulbWidget.update` hides when the setting is Off, invoked triggers included (`lightBulbWidget.ts:311`).
- **Port:** `trigger_code_actions` always calls `update_lightbulb` (`code_action.rs:1110`); the painter doesn't check the mode (`element.rs:4154, 5310-5315`).
- **Repro:** `lightbulb = Off`, Cmd+., Escape. A bulb shows until the caret moves.

#### C.26 Focusing an action ignores `CodeAction::ranges` (P2)
The missing "Context: …" announcement is in §H.14.
- **Monaco:** `onFocus` decorates `action.ranges`, or every diagnostic when there's more than one (`codeActionController.ts:370-391`).
- **Port:** `ranges` (`providers.rs:1112`) is never read; `set_focus` highlights diagnostics only (`code_action.rs:513-529`).
- **Repro:** an action with `ranges` and no diagnostics; focus it in the menu. Nothing is highlighted.

#### C.27 AI code actions aren't sorted last (P3)
- **Monaco:** `isAI` actions come after the others (`contrib/codeAction/browser/codeAction.ts:54-67`).
- **Port:** `sort_items` keys on diagnostics and preferred only (`code_action.rs:227-234`); the module doc's "Not ported: AI actions" (`:11`) is stale.
- **Repro:** a provider returns an AI action before an ordinary one: the port lists it first, and `autoApply: First` picks it.

#### C.28 `editor.codeActionWidget.showHeaders` is missing (P3)
- **Monaco:** `contrib/codeAction/browser/codeActionContributions.ts:25-35`, read at `codeActionController.ts:424`, applied in `codeActionMenu.ts:40-52`.
- **Port:** headers always show (`code_action.rs:434-447`); no option in `options.rs`.
- **Repro:** there's no way to hide the group headers.

#### C.29 The code action menu lacks secondary keys (P3, upstream, not vendored)
- **Monaco:** Cmd+. inside the menu accepts the focused action; Cmd+Up/Down move.
- **Port:** Up, Down, Enter, Cmd+Enter and Escape only (`actions.rs:2121-2134`).
- **Repro:** Cmd+. twice: the second press doesn't apply the first fix.

#### C.30 The menu opened from the lightbulb can cover the bulb (P3)
- **Monaco:** at the click's x, at `bulb bottom + lineHeight/3`, plus a line when the bulb is above the cursor line (`lightBulbWidget.ts:197-214`).
- **Port:** below the cursor row at column 0 (`code_action.rs:1285`).
- **Repro:** click a bulb drawn on the line below the cursor.

#### C.31 The context menu stays open while the wheel scrolls the text (P3)
- **Monaco:** `onMouseWheel` hides it (`contrib/contextmenu/browser/contextmenu.ts:56-66`).
- **Port:** `on_scroll_wheel` (`editor.rs:5754-5766`) never calls `hide_context_menu`.
- **Repro:** right-click, then wheel over the text.

#### C.32 Context menu activation and shadow (P3, upstream, not vendored)
- **Activation:** the port fires items on mouse down (`context_menu.rs:483-495`); Monaco on mouse up, so a press can be dragged off to cancel.
- **Shadow:** GPUI's `.shadow_md()` (`context_menu.rs:389`) rather than `widget.shadow`, so high-contrast themes get a shadow.
- **Repro:** press on an item, drag off, release: the port has already run it.

#### C.33 Rename suggestions use the UI font (P3)
The missing "Received {n} rename suggestions" announcement is in §H.14.
- **Monaco:** candidates use the editor font (`contrib/rename/browser/renameWidget.ts:1039`).
- **Port:** rows use the UI font (`rename.rs:376`), while `list_width` measures with the editor's `char_width` (`rename.rs:321-331`).
- **Repro:** a rename provider returning candidates.

#### C.34 Standalone color picker placement and header (P3)
- **Placement:** Monaco puts it below first when `hover.above` is false (`contrib/colorPicker/browser/standaloneColorPicker/standaloneColorPickerWidget.ts:130`); the port always tries Above (`color_picker.rs:838-841`).
- **Header:** Monaco sizes the picked and original boxes from the body width (`standaloneColorPickerWidget.ts:205-215`); the port keeps the hover's fixed 74px (`color_picker.rs:462`).
- **Repro:** `hover.above = false`, run Show or Focus Standalone Color Picker.

### Find widget and quick input (#332, #340, #343)

#### C.35 Tab and Shift+Tab in the quick input and the rename box indent the document (P1)
- **Monaco:** `tab` and `outdent` need `editorTextFocus` (`browser/coreCommands.ts:2004-2012, 2027-2035`).
- **Port:** the single-line input propagates `Tab` and `Outdent` (`editor.rs:3624-3628, 3700-3704`); neither `QuickInput` (`quick_access.rs:779-792`) nor the rename root (`rename.rs:488-510`) handles them, so the outer editor's handler (`editor.rs:6604-6605`) indents at the hidden cursor. The find widget handles both (`find_widget.rs:1748-1749`).
- **Repro:** F1 or Ctrl+G, Tab, Escape. The cursor line is indented and the edit is in undo. (Read from the dispatch path; not reproduced live.)

#### C.36 There's no FindOptionsWidget: the toggles give no feedback while find is closed (P2)
- **Monaco:** with find closed, Alt+C, W, R and P show the options widget for 2s (`contrib/find/browser/findController.ts:254-280, 525-539`, `findOptionsWidget.ts`/`.css`).
- **Port:** the handlers only flip the option (`editor.rs:6735-6772`).
- **Repro:** find closed, Alt+C. Nothing shows, yet the next search's case setting changed.

#### C.37 The find-in-selection scope isn't re-captured when an input regains focus (P2)
- **Monaco:** focusing an input runs `_updateSearchScope()` (`contrib/find/browser/findWidget.ts:245-260, 856-885`).
- **Port:** set only when the toggle flips or find starts (`find.rs:545-554`).
- **Repro:** select lines 1–5, Cmd+F, Alt+L; select lines 10–20; click into the find input. Monaco moves the scope to 10–20; the port keeps 1–5.

#### C.38 Opening find never shifts the text, so a selection under the widget stays hidden (P2)
- **Monaco:** `_reveal` sometimes skips `adjustEditorScrollTop`, pushing the text below the widget (`findWidget.ts:595-621`).
- **Port:** always compensates the scroll (`find.rs:2090-2102`).
- **Repro:** at the top of a file, select a word at the right end of line 1, Cmd+F. The match stays covered.

#### C.39 Go to Line doesn't switch relative line numbers to absolute (P2)
- **Monaco:** `contrib/quickAccess/browser/gotoLineQuickAccess.ts:132-141`.
- **Port:** `quick_access.rs` never changes `line_numbers`.
- **Repro:** `line_numbers: Relative`, Ctrl+G. The gutter keeps showing distances.

#### C.40 Go to Symbol can't match names that contain spaces (P2)
- **Monaco:** scores the whole query against the name first (`contrib/quickAccess/browser/gotoSymbolQuickAccess.ts:296-305`).
- **Port:** scores the first word against the name and the rest against the container (`quick_access.rs:1261-1292`).
- **Repro:** `@getting started` in Markdown with `# Getting Started`: no symbols.

#### C.41 The find widget lacks its 4px top margin (P3)
- **Monaco:** `contrib/find/browser/findWidget.css:15`.
- **Port:** `widgets.rs:466-471`, `find_widget.rs:1712-1766`.
- **Repro:** Cmd+F: the widget is flush with the editor's top.

#### C.42 Go to Line and Offset parse numbers strictly, not like `parseInt` (P3)
- **Monaco:** `gotoLineQuickAccess.ts:166, 205, 221`.
- **Port:** `quick_access.rs:1157-1159, 1181, 1211`. A JS-compatible `parse_int` exists at `find.rs:2192`.
- **Repro:** `:12.`. Monaco goes to line 12; the port shows the prompt.

#### C.43 Quick access previews use the wrong overview ruler lane (P3)
The missing "Line {0}, column {1}" announcement is in §H.14.
- **Monaco:** the preview's overview mark is in the Full lane (`contrib/quickAccess/browser/editorNavigationQuickAccess.ts:150-156, 216-220`).
- **Port:** the Right lane (`quick_access.rs:1061-1064, 1086-1089`).
- **Repro:** type `:40` and look at the overview ruler.

#### C.44 A seeded regex search string is over-escaped (P3)
- **Monaco:** `escapeRegExpCharacters` (upstream, not vendored), from `findController.ts:334-336`.
- **Port:** `regex::escape` (`find.rs:1672-1673, 1998-1999`) also escapes `-`, `#`, `&` and `~`.
- **Repro:** regex on, select `a-b`, Cmd+F: the input shows `a\-b`.

#### C.45 The find-match foreground colors are swapped (P3)
- **Monaco:** `findWidget.ts:1488-1495`, `findDecorations.ts:286-325`; the decoration used above 1000 matches has no text color.
- **Port:** `find.rs:276-281, 297-302`.
- **Repro:** a theme that sets both find-match foreground colors.

#### C.46 Quick input chrome (P3, upstream, not vendored)
- **Centring:** Monaco centres on the whole editor width; the port on the text area (`widgets.rs:483-490`).
- **Corners:** Monaco rounds them; the port doesn't (`quick_access.rs:768-778`).
- **Input border:** Monaco uses the input and focus border colors; the port `inputOption.activeBorder` (`quick_access.rs:813`).
- **Repro:** open the command palette in an editor with a wide gutter or minimap.

### Inline suggestion toolbar, inline-edit menus, paste and drop, peek (#341)

#### C.47 The always-shown inline toolbar uses `editorWidget.*` colors with no radius or shadow (P2)
Audit 2 #11 and #12 fixed the hover but not this card.
- **Monaco:** `withBorder` adds `monaco-hover monaco-hover-content` (`contrib/inlineCompletions/browser/hintsWidget/inlineCompletionsHintsWidget.ts:204`): `editorHoverWidget.*` colors, the large radius and `shadow-lg` (`inlineCompletionsHintsWidget.css` `.withBorder`, `contrib/hover/browser/hover.css:23-29`).
- **Port:** `with_border` uses `widget_*` colors with no rounding or shadow (`inline_completions/toolbar.rs:444-453`); button hover uses `scrollbar_slider_hover` rather than `toolbar.hoverBackground` (`toolbar.rs:208, 375`; `colors.toolbar_hover` exists, `theme.rs:1398`); the long-distance hint's lower bar uses `widget_foreground` rather than `descriptionForeground` (`inline_completions/long_distance_hint.rs:458` vs `inlineEditsLongDistanceHint.ts:411`).
- **Repro:** `inline_suggest.show_toolbar: Always` in a theme whose hover and widget colors differ.

#### C.48 The paste and drop options chip survives cursor moves (P2)
- **Monaco:** `PostEditWidget` disposes on `onDidChangeCursorPosition` (`contrib/dropOrPasteInto/browser/postEditWidget.ts:86-88`).
- **Port:** `refresh_post_edit` checks only the text version (`drop_paste.rs:972-981`); its comment claims cursor moves are handled, but only Escape (`editor.rs:6973`) and a new drop or paste call `clear_post_edit`.
- **Repro:** two paste providers; paste, then press →. Monaco's button goes; the port's chip stays, and Cmd+. still opens its picker.

#### C.49 No inline progress widget for slow paste and drop providers (P2)
- **Monaco:** `InlineProgressManager.showWhile` shows a spinner after 500ms ("Running paste handlers. Click to cancel and do basic paste", "Running drop handlers. Click to cancel", "Resolving paste edit for '{0}'"); clicking cancels (`contrib/dropOrPasteInto/browser/copyPasteController.ts:379, 400`, `dropIntoEditorController.ts:147`, `contrib/inlineProgress/browser/inlineProgress.ts:114, 138-154`). `inlineProgress` ships in standalone Monaco (`editor.all.ts:29`).
- **Port:** nothing; a slow provider makes Cmd+V do nothing visible, and only Escape cancels. `docs/MONACO_PARITY.md:230` wrongly calls `inlineProgress` "VS Code workbench UI".
- **Repro:** a paste provider that sleeps 3s; paste. Monaco shows a spinner at 0.5s; the port nothing.

#### C.50 Keybinding labels in the inline toolbar and gutter menu are hard-coded (P3)
- **Monaco:** `keybindingService.lookupKeybinding`/`appendKeybinding`, drawn as `KeybindingLabel` keycaps (`inlineCompletionsHintsWidget.ts:146, 344-358`, `.css:29-38`; `contrib/inlineCompletions/browser/view/inlineEdits/components/gutterIndicatorMenu.ts:193, 253-263`).
- **Port:** literal "Tab", "⌘→"/"Ctrl+RightArrow", "Previous (⌥[)", "Next (⌥])" (`inline_completions/toolbar.rs:306-361`) and "Tab", "Shift+Tab", "Escape" (`inline_completions/inline_edits_view.rs:996-1008`). They ignore rebinding and `keybinding_label.rs`, and macOS shows "Shift+Tab" not "⇧Tab".
- **Repro:** rebind accept-word and hover the toolbar.

#### C.51 The toolbar "⋯" menu and the inline-edit gutter menu don't look like Monaco's menus (P3)
- **"⋯" menu:** Monaco uses the standard context menu; the port uses `widget_*` colors, a `widget_border` separator, 6×2px rows and no radius (`toolbar.rs:391-430`), unlike its own context menu (`context_menu.rs:380-470`).
- **Gutter menu:** Monaco rows have 16px codicons (`gutterIndicatorMenu.ts:94-140, 239-249`), a `descriptionForeground` 13px/600/28px header (`:207-217`), `editorHoverBorder` separators (`:289-300`) and a footer action bar for the link (`:157-168`). The port has no icons, a 0.8-opacity header, `widget_border` separators and the link as a row (`inline_edits_view.rs:805-878`).
- **Repro:** open the "⋯" menu beside the right-click menu.

#### C.52 The post-edit chip doesn't look like Monaco's button (P3)
`book/src/differences.md` documents "a small chip" but gives no reason, and codicons are now available.
- **Monaco:** a `$(insert)` icon button in `button.background`/`button.foreground`, 2px padding, 4px radius, `shadow-lg`, `widget.border` (`postEditWidget.css`, `postEditWidget.ts:100-105`); tooltip "Show paste options... (⌘.)" (`:95-97`); placed `BELOW` only (`:118-122`).
- **Port:** an 11px "Paste options ▾" text chip in widget colors, no tooltip, placed Below then Above (`drop_paste.rs:451-475, 919-932`).
- **Repro:** paste with two providers registered.

#### C.53 The peek title for models without a URI or with an `inmemory:` URI (P3)
- **Monaco:** for `inmemory:` URIs the title is "References" whatever the kind (`contrib/gotoSymbol/browser/peek/referencesWidget.ts:567-571`); the meta is "{kind} ({n})" (`referencesController.ts:164`).
- **Port:** with no URI, the title is the kind, giving "Definitions - Definitions (2)" (`peek.rs:879-884`); `inmemory://model/1` is split into "1" and "model" (`peek.rs:197-204`).
- **Repro:** Shift+F12 or Alt+F12 in the playground.

## D. Navigation, folding, scrolling and sticky scroll

The audit 2 fixes for sticky scroll (§C.F), the minimap (§C.I) and scrollbars and the overview ruler (§C.J) hold, except sticky focus (D.29).

Checked and matching: the reveal math in `view/reveal.rs` (sticky padding, minimal and mouse rules, the horizontal reveal), smooth scrolling (125ms `easeOutCubic`), `fastScrollSensitivity`, `mouseWheelZoom`, `alwaysConsumeMouseWheel`, `handleMouseWheel`, the overview ruler, `editorScroll by: line`, `revealLine` arguments, goto symbol (alternative commands, `gotoLocation.multiple*`, peek modes, `symbolHighlight`, the "No definition found" message, Escape), the definition link preview, the word highlighter, cursor undo, anchor commands, quick outline ordering, every fold command and its arguments, `foldingHighlight`, `unfoldOnClickAfterEndOfLine`, `showFoldingControls`, fold limits and the memento checksum, and the sticky scroll and minimap layouts.

### Folding state (#324)

#### D.1 A cursor that lands inside a folded region never unfolds it (P1)
Also found by the editing audit (B9).
- **Monaco:** any cursor change schedules `revealCursor` 200ms later; if a selection's start line is hidden, every collapsed region containing it is expanded (`contrib/folding/browser/folding.ts:247`, `:364-394`).
- **Port:** `cursors_changed` (`editor.rs:2658-2743`) has no such step. Outside `view/folding.rs`, `FoldState::expand` is called only when restoring view state (`folding_commands.rs:478`); `find.rs`, `goto.rs`, `multi_cursor.rs` and `api.rs` never touch folds. The only automatic unfold is on edit (`editor.rs:4104-4109`). The cursor is drawn at the end of the fold header (`view/view_lines.rs:379`).
- **Repro:** fold a function body, then Cmd+F a word inside it and press Enter, Go to Line into it, Go to Definition of a symbol in it, Jump to Bracket from the folded `{`, Cmd+D onto an occurrence in it, or `set_selections` into it. The target stays hidden.

#### D.2 Fold ranges are plain row numbers that edits don't move (P1)
- **Monaco:** regions are model decorations, so icons and folds move with the text (`contrib/folding/browser/foldingModel.ts:138-157`); commands wait for the recomputed model (`folding.ts:556-568`).
- **Port:** `FoldState.ranges` and `computed` are rows replaced only by `set_ranges` (`view/folding.rs:786-795`); recomputing waits 200ms, restarted by every version (`editor.rs:4170-4176`). Chevrons (`element.rs:3519-3528`) and commands (`folding_commands.rs:37-40`) use the stale rows.
- **Repro:** press Enter a few times near the top of a file: the chevrons below stay on their rows. Insert three lines and press Cmd+K Cmd+0 within 200ms: the wrong lines fold and stay folded.

#### D.3 Any edit that touches a folded region unfolds it (P2)
- **Monaco:** lines inserted into a hidden area stay hidden (`common/viewModel/viewModelLines.ts:358`); on recompute a region stays collapsed unless its length changed and a selection starts in it (`contrib/folding/browser/foldingRanges.ts:339`, `foldingModel.ts:171`).
- **Port:** `expand_intersecting` runs on every edit touching hidden rows (`editor.rs:4104-4109`, `view/folding.rs:1026-1038`).
- **Repro:** fold several functions, then Replace All or Format Document: everything unfolds.

#### D.4 A collapsed region doesn't follow its recomputed range (P2)
- **Monaco:** `sanitizeAndMerge` takes the new range when the start line matches (`foldingRanges.ts:331-342`).
- **Port:** collapsed anchors are separate from ranges; `set_ranges` doesn't reconcile them, and only `prune` drops one-row regions (`view/folding.rs:763`, `:998-1009`).
- **Repro:** fold an indented block, then indent the line after it into the block. The new line stays visible under a collapsed chevron.

#### D.5 Crossing fold ranges are kept (P2)
- **Monaco:** `update` always runs `sanitizeAndMerge`, dropping a range not inside its parent (`foldingRanges.ts:371-379`) and provider ranges conflicting with a manual range that ends later (`:354-366`).
- **Port:** with no manual ranges, `ranges = computed` from `sanitize_ranges`, crossing ranges included (`view/folding.rs:278-286`, `:806-830`).
- **Repro:** a provider returning rows 0–5 and 3–8 gives two chevrons; Monaco shows one.

### Folding recompute and polish (#339)

#### D.6 `foldingImportsByDefault` differs (P2)
- **Monaco** (`folding.ts:216`, `:323-329`): keeps trying on each computation until an import range has been folded, sets imports collapsed (`setCollapsedAllOfType(..., true)`) and saves `foldedImports` in the view state.
- **Port** (`editor.rs:4196-4214`): runs only on the first computation (`ranges_version().is_none()`), calls `toggle` (expanding imports already collapsed), and `FoldingViewState` has no `foldedImports`.
- **Repro:** register a folding provider after the editor opens: imports never fold. Restore a view state with folded imports right after `set_model`: they unfold.

#### D.7 Folds aren't recomputed when options, language or providers change (P2)
- **Monaco:** recomputes on `folding`, `foldingMaximumRegions` or `foldingStrategy` changes, provider registration and unregistration, and language configuration changes (`folding.ts:155-173`, `:249-250`).
- **Port:** only a new text version or a bumped `FoldingRangesEpoch` recomputes (`editor.rs:4142-4156`); `update_options` (`:1189-1235`), the language-change handlers (`:2493-2517`) and provider registration (`providers.rs:2936-2954`, `:3168`) don't bump it.
- **Repro:** set `folding_strategy: Indentation`, or register a folding provider after opening. Nothing changes until the next edit.

#### D.8 "Has types" is true for providers that give no kinds (P2)
- **Monaco:** `fromFoldRanges` drops types when no range has a kind (`foldingRanges.ts:260-277`); Fold All Block Comments and Fold All Regions then match the comment and region-marker patterns (`folding.ts:871-883`, `:903-915`).
- **Port:** `has_types` is true whenever a provider answered (`editor.rs:4289-4291`).
- **Repro:** a provider returning ranges without kinds; Cmd+K Cmd+/ folds nothing.

#### D.9 Fold commands reveal minimally instead of centring (P3)
- **Monaco:** reveals the selection start centred if outside the viewport (`folding.ts:562-565`).
- **Port:** the minimal cursor autoscroll; go-to-fold uses `RevealType::Simple` (`folding_commands.rs:242`).
- **Repro:** Go to Parent Fold when the parent is far above.

#### D.10 Turning `folding` off keeps the folds (P3)
- **Monaco:** discards them (`folding.ts:156-159`, `:231`).
- **Port:** keeps them and only stops hiding rows (`editor.rs:4121`), so they return when folding is turned back on; go-to-fold, remove-manual-ranges and click commands ignore `options.folding` (`folding_commands.rs:230-311`).
- **Repro:** fold, set `folding: false`, then `true`.

#### D.11 Restoring a view state moves cursors out of folds (P3)
- **Monaco:** skips `adjustSelections` while restoring (`folding.ts:218`, `:353`).
- **Port:** moves the restored cursors (`folding_commands.rs:484`).
- **Repro:** save a view state with the cursor inside a fold that's collapsed, then restore it.

#### D.12 Orphaned folds don't get the manual icon (P3)
- **Monaco:** recovered collapsed regions count as manual for the icon (`contrib/folding/browser/foldingDecorations.ts:152`, `foldingModel.ts:145`).
- **Port:** checks `range.manual` only (`element.rs:3527`). Visible only with a custom icon.

#### D.13 The fold highlight doesn't shade the minimap (P3)
- **Monaco:** `foldingDecorations.ts:26`, `:47`.
- **Port:** `minimap.rs` has no fold background.
- **Repro:** fold a region with the minimap on.

#### D.14 Fold chevron mouse details (P3)
- **Monaco** (`folding.ts:417`, `:433-471`): toggles on mouse-up when down and up are on the same line; a 4px dead zone at the start of the folding column; any text click at the collapsed line's last column unfolds.
- **Port** (`editor.rs:4736-4755`): toggles on mouse-down, no dead zone, only the `⋯` box unfolds.
- **Repro:** press on a chevron, drag to another line, release.

#### D.15 Folding limit reporting (P3)
- **Monaco:** `FoldingController.limitReporter` (`computed`/`limited` and an event) and `_executeFoldingRangeProvider` (`folding.ts:517-544`, `:1277`).
- **Port:** discards the "limited" flag (`editor.rs:4290`, `:4313`) and has neither API.

### Reveal and autoscroll (#323)

#### D.16 Clicking a fold chevron scrolls back to the cursor (P1)
- **Monaco:** a chevron click reveals only the clicked line (`revealPositionInCenterIfOutsideViewport`, `folding.ts:506-507`).
- **Port:** `click_fold` → `after_fold_change` → `cursors_changed(true)` (`folding_commands.rs:364`, `:369-372`), which sets `autoscroll_requested` for unchanged selections from a non-API source (`editor.rs:2687-2692`); the element reveals the primary cursor (`element.rs:3082-3107`).
- **Repro:** cursor on line 1, wheel to line 300, click a chevron there. The view jumps to line 1.

#### D.17 `editorScroll` with `revealCursor: true` undoes its own scroll (P1)
- **Monaco:** computes the new scroll top, moves the cursor into that viewport (padded by `cursorSurroundingLines`, `findPositionInViewportIfOutside`), then sets the scroll position without a cursor reveal (`browser/coreCommands.ts:1354-1373`).
- **Port** (`editor_commands.rs:640-693`): changes `scroll_top` (`navigation_history.rs:212-237`), then runs `cursorMove viewPortIfOutside` against last frame's `visible_rows` (`editor_commands.rs:549-578`, `api.rs:969`). The cursor doesn't move, but `set_cursor_states(.., true)` → `cursors_changed(true)` still requests an autoscroll (`editor.rs:2677-2693`), and the next frame scrolls back.
- **Repro:** cursor on line 1, `execute_command("editorScroll", {to: "down", by: "page", revealCursor: true})`. Monaco scrolls a page and puts the cursor on the first visible line; the port doesn't move.

#### D.18 Cursor commands reveal only the primary cursor, not all of them (P1)
Found by the editing audit (B2) and the navigation audit (2.3).
- **Monaco:** every cursor-move command ends with `revealAllCursors` (`browser/coreCommands.ts:617`, `:686`, `:1001`, …), which reveals the box from the first selection to the last if it fits (`common/cursor/cursor.ts:145-157`; `browser/viewParts/viewLines/viewLines.ts:701-711`, `:748-753`). Add Cursor Above/Below reveal the top- or bottom-most cursor (`contrib/multicursor/browser/multicursor.ts:93,145`), Cmd+D reveals the new match (`_applySessionResult`, `:584`), and column select its newest cursor (`browser/coreCommands.ts:419-440`).
- **Port:** the autoscroll reveals `primary_view_line` only (`element.rs:3082-3107`). `reveal_all_cursors` (`api.rs:941-965`) exists but no command calls it, and nothing in `multi_cursor.rs`, `add_cursor_above/below` (`editor.rs:3291-3307`) or `column_select` (`editor.rs:3768-3775`) asks for a reveal.
- **Repro:** hold Cmd+Alt+Down past the bottom of the viewport: Monaco follows the new cursors, the port doesn't scroll. Cmd+D with the next match off screen: same. Cursors on lines 10 and 40 of a 30-line viewport, line 10 at the top, press Down: Monaco shows line 41; the port leaves it off screen.

#### D.19 PageUp/PageDown page size and scrolling differ (P2)
Found by the editing audit (B4) and the navigation audit (2.2).
- **Monaco:** `pageSize = max(1, floor(height / lineHeight) - 2)` (`common/cursorCommon.ts:129`); `cursorPageDown` moves the cursors `pageSize` view lines and then only calls `revealAllCursors` (`browser/coreCommands.ts:659-687`, `:860-875`). `scrollPageDown`/`halfPage` use the same `pageSize` (`:1404-1413`), leaving two lines of overlap.
- **Port:** `page_rows()` has no `- 2` (`element.rs:1838-1840`), used for the cursor move (`cursor/movement.rs:100-107`), `scroll_pages` and `editorScroll` (`navigation_history.rs:226-229`, `editor_commands.rs:642`). `move_cursors` also scrolls by a page itself (`editor.rs:3246-3258`; its "Like Monaco" comment is wrong).
- **Repro:** 20-line viewport, caret on line 1, PageDown. Monaco: caret to line 19, no scroll. Port: caret to line 21, a full page scrolled. Cmd+PageDown keeps no overlap lines.

### Scroll command polish (#355)

#### D.20 `scrollEditorBottom` scrolls past the last line (P3)
- **Monaco:** scroll top at line `lineCount - pageSize` (`browser/coreCommands.ts:1396-1402`).
- **Port:** `scroll_top = 1e9`, clamped (`navigation_history.rs:232-237`); with `scrollBeyondLastLine` the last line ends at the top of an almost empty viewport.
- **Repro:** run `scrollEditorBottom` in a long file.

#### D.21 Scroll-by-line and scroll-by-page commands are never smooth (P3)
- **Monaco:** `_runVerticalEditorScroll` and the horizontal variant use `ScrollType.Smooth` (`browser/coreCommands.ts:1372`, `:1418`).
- **Port:** `scroll_by` sets `scroll_top` without `smooth_scroll_requested` (`navigation_history.rs:212-217`); only `editorScroll by: "line"` animates (`editor_commands.rs:664`).
- **Repro:** `smooth_scrolling: true`, Cmd+PageDown.

#### D.22 The `revealLine` command also scrolls horizontally (P3)
- **Monaco:** `RevealLine` passes `revealHorizontal = false` (`browser/coreCommands.ts:1877`).
- **Port:** `reveal_line_command` → `reveal_range` with `horizontal: true` (`editor_commands.rs:702-722`, `api.rs:752-765`), scrolling left through `scroll_left_to_reveal` (`element.rs:5928-5968`).
- **Repro:** scroll right on a long line, `revealLine` a visible short line: the port jumps to column 0.

#### D.23 Long smooth scrolls don't use Monaco's composed animation (P3)
- **Monaco:** past 2.5 viewports it eases through the first and last 0.75 viewport and skips the middle (`createComposed(…, 0.33)`, `base/common/scrollable.ts:458-473`), starting 10ms early (`:507-513`).
- **Port:** one `ease_out_cubic` over the distance (`view/animation.rs:214-251`).
- **Repro:** `smooth_scrolling`, Cmd+End in a long file.

### Sticky scroll and section headers (#337)

#### D.24 Switching models can show the previous model's sticky lines and section headers (P1)
- **Monaco:** a model change rebuilds the sticky provider (`contrib/stickyScroll/browser/stickyScrollProvider.ts:97-102`) and recomputes section headers (`contrib/sectionHeaders/browser/sectionHeaders.ts:40-45`).
- **Port:** `set_model_inner` (`editor.rs:1002-…`) resets folds and the minimap cache but not `self.sticky` or `self.section_headers`. Both caches key on buffer version, language and options (`sticky_model.rs:340-356`, `section_headers.rs:196-206`), so two unedited models of one language share a key and `refresh_*` returns early; the in-flight guards (`sticky_model.rs:409`, `section_headers.rs:239`) pass too.
- **Repro:** two unedited Rust files A and B in one editor; scroll A into a function; `set_model(B)`. The sticky lines are A's rows drawn with B's text, and B's minimap shows A's `MARK:` labels.

#### D.25 Sticky lines are drawn from stale rows while the model recomputes (P3)
- **Monaco:** renders only while the sticky model's version equals the text model's (`contrib/stickyScroll/browser/stickyScrollController.ts:557-559`).
- **Port:** paints old rows against new text during the 300ms delay (`element.rs:3689-3746`).
- **Repro:** Format Document inside a nested function.

#### D.26 Clicking a sticky line's number moves the cursor (P3)
- **Monaco:** a plain click acts only on the content span (`data-sticky-is-line`, `stickyScrollController.ts:307-310`, `stickyScrollWidget.ts:489` vs `:499`).
- **Port:** `sticky_click` falls back to `(row, 0)` (`sticky_model.rs:517-518`).
- **Repro:** click a sticky line's number.

#### D.27 Clicking past the end of a sticky line's text goes to the line end (P3)
- **Monaco:** column 1, since there's no leaf node (`stickyScrollWidget.ts:363-366`, controller `:312-320`).
- **Port:** `column_for_x` clamps to the line end (`element.rs:1825-1835`).
- **Repro:** click right of a sticky line's text.

#### D.28 Wrong pointer over sticky lines (P3)
- **Monaco:** `.sticky-line-content:hover { cursor: pointer }` (`stickyScroll.css`); visible sticky chevrons get `cursor: pointer` (`stickyScrollWidget.ts:575`).
- **Port:** sticky lines aren't in the cursor-style lists (`element.rs:2106-2142`): an I-beam on lines, an arrow on chevrons, and gutter fold controls under the sticky area still claim a hand.
- **Repro:** hover a sticky line.

#### D.29 Sticky keyboard focus survives blur and lets typing through (P3)
Left over from audit 2 §C.F 27.
- **Monaco:** focus is on the sticky DOM node; blur disposes it (`stickyScrollController.ts:126-137`), and typing doesn't reach the text.
- **Port:** `on_focus_out` doesn't clear `sticky.focused` (`editor.rs:770-777`); a typed character edits the text, then clears focus (`editor.rs:2741`).
- **Repro:** focus sticky scroll, click outside the editor: the outline stays. Focus it and type a letter: the text changes.

#### D.30 The sticky folding model includes manual fold ranges (P3)
- **Monaco:** its own `SyntaxRangeProvider` over the folding providers, then indentation (`contrib/stickyScroll/browser/stickyScrollModelProvider.ts:386-416`).
- **Port:** converts `self.folds.ranges()`, manual ranges included (`sticky_model.rs:307-320`).
- **Repro:** "Create Folding Range from Selection" makes that range a sticky scope.

#### D.31 `minimap.autohide: "scroll"` reacts to the wrong changes (P3)
- **Monaco:** shows on `scrollTopChanged || scrollHeightChanged` (`browser/viewParts/minimap/minimap.ts:1469-1471`).
- **Port:** on top or left changes (`element.rs:3488-3496`).
- **Repro:** scroll horizontally: the minimap shows. Add lines: it doesn't.

#### D.32 Minimap section headers don't move with edits (P3)
- **Monaco:** tracked decorations (`contrib/sectionHeaders/browser/sectionHeaders.ts:200-214`).
- **Port:** plain `Range<PointUtf16>` (`section_headers.rs:24-34`), on the wrong line until the 250ms recompute.
- **Repro:** insert lines above a `// MARK:` comment.

### Navigation polish (#350)

#### D.33 The Cmd/Ctrl+hover definition link uses the wrong theme color (P2)
- **Monaco:** `.goto-definition-link { text-decoration: underline; color: var(--vscode-editorLink-activeForeground) }` (`contrib/gotoSymbol/browser/link/goToDefinitionAtPosition.css`).
- **Port:** `inputOption.activeBorder` (`definition.rs:308`); `editorLink.activeForeground` is parsed as `link_active_foreground` (`theme.rs:278`) but unused.
- **Repro:** Cmd-hover a symbol in the dark theme: #007ACC instead of #4E94CE.

#### D.34 F12 through symbol results highlights each target (P3)
- **Monaco:** `SymbolNavigationService.revealNext` opens `Range.collapseToStart(reference.range)` with no `symbolHighlight` flash (`contrib/gotoSymbol/browser/symbolNavigation.ts:107-133`).
- **Port:** `go_to_next_symbol_from_result` calls `open_location(&location, true)`, which highlights, and uses `targetSelectionRange` (`goto.rs:480-492`).
- **Repro:** Go to References with several results, then F12 through them.

#### D.35 The selection anchor looks different (P3)
The missing "Anchor set at L:C" announcement is in §H.14.
- **Monaco:** an empty-range decoration drawn as a 2px `#007ACC` bar, with a "Selection Anchor" hover message (`contrib/anchorSelect/browser/anchorSelect.ts:44-63`, `anchorSelect.css`).
- **Port:** a one-character box bordered in the cursor color, with no hover (`navigation_history.rs:138-166`).
- **Repro:** Cmd+K Cmd+B.

## E. Diff editor and multi diff editor

The diff algorithms are at parity: all 58 fixture directories pass for both computers (`tests/diffing.rs`), and the `defaultLinesDiffComputer` and legacy `diffComputer` suites are ported (`diff/tests.rs`, `diff/legacy_tests.rs`). `cargo test --test diff_editor --test multi_diff_editor` passes (60 and 20 tests). Every audit 2 §D item re-checked is fixed. Not ported from `browser/widget/diffEditorWidget.test.ts`: "width based layout" (the temporary inline mode, a documented gap) and "compact disclosures" (the cards variant, #299). From `multiDiffEditorWidget.test.ts`, "models bottom padding as trailing scroll content" can't be ported until E.24 is.

### Diff results, revert and alignment (#338)

#### E.1 `normalizeDocumentDiff` isn't ported (P2)
- **Monaco:** the view model normalizes every diff (`browser/widget/diffEditor/diffEditorViewModel.ts:307`, `:381-408`): an inner change from column 1 to the line's max column on both sides, not on the last line, ends at `(endLine + 1, 1)`. So `char-insert`/`char-delete` (`shouldFillLineOnLineBreak: true`, `browser/widget/diffEditor/registrations.contribution.ts:67,84`) paint to the right edge; such changes don't qualify for the true inline view (`allowsTrueInlineDiffRendering`, `components/diffEditorViewZones/diffEditorViewZones.ts:647-655`) or as "simple" for `compactMode`; and revert sees the line break as part of the change.
- **Port:** `DiffEditor::compute_diff` stores the computer's result as is (`diff_editor.rs:3224-3253`); nothing in `src/diff*` normalizes. `should_fill_line_on_line_break` is set (`diff_editor.rs:5878`) but never fires for these changes.
- **Repro:** diff `a\nfoo\nb` against `a\nbar\nb`, `ignore_trim_whitespace` off (fixture `issue-131091` has `[9,1 -> 9,2 EOL]`). Monaco's char highlights on line 2 span the width; the port's stop after `bar`. With `use_true_inline_view` in the inline view, Monaco shows the deleted-lines zone; the port shows struck-through `foo` before `bar`.

#### E.2 Revert never reverts a pure deletion, nor a change ending on the cursor line (P2)
- **Monaco:** `revertFocusedRangeMappings` uses `LineRange.fromRange(selection)` (`[start, end)`, empty for a cursor) and keeps every mapping whose `modified.intersect(selectedRange)` is defined (`browser/widget/diffEditor/diffEditorWidget.ts:696-722`); touching ranges intersect (`common/core/ranges/lineRange.ts:157-164`).
- **Port:** `revert_focused_range_mappings` uses `[row+1, row+2)` and drops empty intersections (`.is_some_and(|r| !r.is_empty())`, `diff_editor.rs:4810-4837`). A deletion's modified range is always empty.
- **Repro:** delete lines 3–4 of the original, cursor on the modified line just below the padding, `diff_editor.trigger("diffEditor.revert")`. Monaco restores the lines; the port does nothing.

#### E.3 The selection toolbar picks up changes the selection only touches (P3)
- **Monaco:** `Range.areIntersecting` (`browser/widget/diffEditor/features/gutterFeature.ts:95`, `revertButtonsFeature.ts:99`) is strict (`common/core/range.ts:427-440`).
- **Port:** `selected_changes` uses `a.start <= b.end && b.start <= a.end` (`diff_editor.rs:4857-4858`).
- **Repro:** select text ending exactly where an inserted span begins. The port offers "Revert Selection" for the untouched insertion.

#### E.4 Comparing a moved block survives cursor moves and follows the block by index (P3)
- **Monaco:** `setActiveMovedTextFromCursor` clears `movedTextToCompare` once the active move differs (`browser/widget/diffEditor/features/movedBlocksLinesFeature.ts:247-249`); each new diff looks the compared move up by modified range (`diffEditorViewModel.ts:236,255,320`).
- **Port:** `update_active_move` never touches `move_to_compare` (`diff_editor.rs:3806-3828`); `apply_diff` keeps the old index unless out of bounds (`diff_editor.rs:3300-3306`).
- **Repro:** `show_moves`, click Compare, click into unrelated code: the port stays in compare mode. An edit that adds a move above makes it compare a different block.

#### E.5 Sub-editors keep `allow_variable_line_heights`, so decoration line heights break alignment (P2)
- **Monaco:** `_adjustOptionsForSubEditor` forces `allowVariableLineHeights = false` and `allowVariableFonts = false` (`browser/widget/diffEditor/components/diffEditorEditors.ts:190-192`).
- **Port:** `apply_editor_options` doesn't set it (`diff_editor.rs:2518-2571`), so it stays `true` (`options.rs:2159`); `element.rs:2916-2920` applies decoration `line_height`, while `alignment_zones` assumes uniform heights (`diff_editor.rs:3377-3402`).
- **Repro:** a side-by-side diff with a `line_height: Some(2.0)` decoration above a change. Every line below is offset against the original.

#### E.6 No `StableEditorScrollState` around realignment, so the modified side jumps (P2)
- **Monaco:** captures and restores `StableEditorScrollState` around applying the modified editor's view zones (`browser/widget/diffEditor/diffEditorWidget.ts:390-399`).
- **Port:** `apply_alignment_zones` removes and re-adds every zone (`diff_editor.rs:3577-3674`); `add_zone`/`remove_widget` don't compensate (`editor.rs:1506-1534`, `:1809`), and `schedule_alignment` (`diff_editor.rs:3548-3566`) and `apply_diff` keep the pixel `scroll_top`.
- **Repro:** scroll halfway down a file with many changes, then toggle `ignore_trim_whitespace` (or edit the original with `original_editable`). The port's content jumps by the padding difference.

### Hidden unchanged regions (#352)

#### E.7 Touching regions aren't merged, so two "N hidden lines" bars can stack (P2)
- **Monaco:** the "collapse touching unchanged ranges" autorun merges regions whose hidden ranges touch, except while dragging (`diffEditorViewModel.ts:93-150`).
- **Port:** `update_unchanged_regions`/`transfer_state` can split a region around a previously visible range (`diff_editor.rs:4252-4281`, `diff_editor/unchanged.rs:159-188`, `:268-302`); nothing merges them (`collapse_all_unchanged_regions` zeroes each, `diff_editor.rs:4584-4589`).
- **Repro:** `hide_unchanged_regions`, two changes ~30 lines apart; revert the first from the gutter, then Collapse All Unchanged Regions. The port shows two bars back to back.

#### E.8 The bar disappears mid-drag once the region is fully revealed (P3)
- **Monaco:** `_shouldHideControls` is `visibleTop + visibleBottom === lineCount && !isDragged` (`diffEditorViewModel.ts:512-513`).
- **Port:** `apply_unchanged` skips regions with `should_hide_controls()` regardless of `unchanged_drag` (`diff_editor.rs:4385-4388`).
- **Repro:** drag a bar's top edge down past the region's end.

#### E.9 The lower scroll anchor counts model lines, so it drifts with word wrap (P3)
- **Monaco:** `_runWithLowerScrollAnchor` measures `getTopForLineNumber(anchor)` before and after (`browser/widget/diffEditor/features/hideUnchangedRegionsFeature.ts:581-596`).
- **Port:** `with_lower_scroll_anchor` uses `line_height * (hidden before - after)` (`diff_editor.rs:4498-4522`).
- **Repro:** `diff_word_wrap` on, long unchanged lines, click a bar's bottom edge: the text below shifts.

#### E.10 Bar edge geometry and drag highlight (P3)
Left over from audit 2 §D #12.
- **Monaco:** 4px edge bands straddling the bar's top and bottom (`.diff-hidden-lines` `translate(0,-10px)`, `.top` `translate(0,4px)`, `.bottom` `translate(0,-6px)`, `browser/widget/diffEditor/style.css:10-33`, `:52-58`); the dragged edge keeps `focusBorder` through `.dragging` (`:17-21`).
- **Port:** 6px edges inside the 24px bar over the label (`UNCHANGED_EDGE_HEIGHT`, `diff_editor.rs:990`, `:1035-1045`, `:1149-1172`); the highlight is hover-only and can drop during a captured drag.
- **Repro:** drag a bar edge and watch the highlight.

### Context keys, events and polish (#354)

#### E.11 Monaco's diff context keys are missing (P3)
- **Monaco:** `isInDiffEditor` (`diffEditorWidget.ts:203`), `isInDiffLeftEditor`/`isInDiffRightEditor` (`diffEditorEditors.ts:107`, `:118`), `diffEditorInlineMode`, `diffEditorRenderSideBySideInlineBreakpointReached`, `hasChanges`, `diffEditorOriginalWritable`, `diffEditorModifiedWritable` and the uri keys (`diffEditorWidget.ts:262-305`); `inMultiDiffEditor` and `multiDiffEditorAllCollapsed` (`browser/widget/multiDiffEditor/multiDiffEditorWidgetImpl.ts:220`, `:239`).
- **Port:** only `DiffEditor`, `comparingMovedCode` and `isEmbeddedDiffEditor` (`diff_editor.rs:6150-6160`).
- **Repro:** `add_command("…", h, Some("isInDiffEditor && hasChanges"))` never fires.

#### E.12 Alt+F5 / Shift+Alt+F5 are documented as Monaco keys (P3)
- **Monaco:** `browser/widget/diffEditor/commands.ts` binds only F7/Shift+F7 (`:215`, `:238`) and Escape; `goToDiff` has no keybinding.
- **Port:** `bind_keys` adds `alt-f5`/`shift-alt-f5` (`diff_editor.rs:127-132`), and `book/src/diff-editor.md:18` lists them as Monaco's. `docs/MONACO_AUDIT.md:158` flagged them already.
- **Fix:** remove them, or document them as a deliberate addition.

#### E.13 `go_to_diff` focuses the modified editor (P3)
- **Monaco:** `_goTo` only sets the position and reveals (`diffEditorWidget.ts:724-757`).
- **Port:** `window.focus(&self.modified…)` (`diff_editor.rs:5559`).
- **Repro:** original side focused, call `go_to_diff`: focus jumps.

#### E.14 No diff-level content size event (P3)
- **Monaco:** `IDiffEditor.onDidContentSizeChange` with `contentHeight` (the larger side) and `contentWidth` (original + modified + 30) (`diffEditorWidget.ts:71`, `diffEditorEditors.ts:128-137`).
- **Port:** `DiffEditorEvent` has `DiffUpdated`, `ModelChanged` and `AccessibilitySignal` only (`diff_editor.rs:307-318`); `content_height()` exists (`:2945`) but nothing notifies.
- **Repro:** an auto-height diff editor can't resize when content changes.

#### E.15 The diff overview slider has no hover color (P3)
- **Monaco:** `.diffViewport:hover` uses `scrollbarSlider.hoverBackground` (`browser/widget/diffEditor/style.css:437-439`).
- **Port:** `scrollbar_slider`, or the active color while dragging (`diff_editor.rs:4114-4119`); `.hover(|style| style)` is a no-op (`:4171`).
- **Repro:** hover the diff overview slider.

#### E.16 High-contrast change borders are missing on whole lines and solid instead of dashed (P3)
- **Monaco:** `.line-insert`/`.char-insert` and `.line-delete`/`.char-delete` get `1px solid insertedTextBorder`/`removedTextBorder`, dashed in `hc-black`/`hc-light` (`browser/widget/diffEditor/style.css:381-399`).
- **Port:** `line()` decorations set no border (`diff_editor.rs:5850-5859`); only char decorations do (`:5873-5880`), always `BorderStyle::Solid` (`element.rs:5374`).
- **Repro:** HC dark theme: Monaco outlines changed lines with dashes; the port outlines changed characters with solid lines.

#### E.17 `split_view_default_ratio` isn't clamped (P3)
- **Monaco:** `clampedFloat(options.splitViewDefaultRatio, 0.5, 0.1, 0.9)` (`browser/widget/diffEditor/diffEditorOptions.ts:205`).
- **Port:** copied as is (`diff_editor.rs:2414-2416`) and used by `compute_sash_left` (`:2089-2102`).
- **Repro:** a ratio of 0.98 puts the sash 100px from the right; Monaco caps it at 90%.

#### E.18 `max_file_size` is enforced, but Monaco's widget ignores the option (P3, decide and document)
- **Monaco:** `maxFileSize` is only validated (`diffEditorOptions.ts:209`); nothing under `vs/` reads it.
- **Port:** `compute_diff` skips the diff above the limit (`diff_editor.rs:3198-3209`); `MONACO_PARITY.md` lists the option as ported.
- **Fix:** keep it and list it in `differences.md`, or remove the cut-off.

### Multi diff editor (#346)

#### E.19 Updating the file list throws away all per-file state (P2)
- **Monaco:** `IMultiDiffEditorModel.documents` is observable (`browser/widget/multiDiffEditor/model.ts:13`), mapped with `mapObservableArrayCached` (`multiDiffEditorViewModel.ts:88-95`); existing items keep their collapsed state, view state and height, and the scroll stays.
- **Port:** `set_documents` rebuilds every `Item` (expanded, 500px, no view state) and resets the scroll, active item and pending reveals (`multi_diff_editor.rs:616-669`).
- **Repro:** collapse a file, scroll down, add a file with `set_documents`: the port jumps to the top with everything expanded.

#### E.20 No first-change selection when files load (P2)
- **Monaco:** the "Initialize first change" autorun runs `_navigateToChange('next', !preserveFocus)` once loading is done and nothing is active (`multiDiffEditorWidgetImpl.ts:293-330`).
- **Port:** neither `new` nor `set_documents` navigates (`multi_diff_editor.rs:574-669`).
- **Repro:** open a multi diff editor whose first change is far down the first file. Monaco scrolls to it; the port shows the top.

#### E.21 No "No Changed Files" / "Loading..." placeholder (P3)
- **Monaco:** a centred `.placeholder` (`multiDiffEditorWidgetImpl.ts:262-276`, `browser/widget/multiDiffEditor/style.css:20`).
- **Port:** an empty list (`multi_diff_editor.rs:2259` onward); documents can't be loading.
- **Repro:** `set_documents(vec![])`.

#### E.22 Per-file diff options, option changes and context keys are missing (P3)
- **Monaco:** `IDocumentDiffItem` has `options`, `onOptionsDidChange` and `contextKeys` (`model.ts:37-40`), merged into that file's editor and reapplied on change (`diffEditorItemTemplate.ts:372-377`, `:442`, `:463-480`).
- **Port:** `MultiDiffDocument` has `label`, `original_label`, `original`, `modified` and `binary` (`multi_diff_editor.rs:170-185`); every file shares `MultiDiffEditor::options` (`:918-956`).

#### E.23 An explicit `hideOriginalLineNumbers: false` can't override the variant (P3)
- **Monaco:** `override ?? options.hideOriginalLineNumbers ?? variant ?? false` (`diffEditorItemTemplate.ts:377`); the vendored test "resolves original line number visibility from variant and explicit options" covers it.
- **Port:** `options.hide_original_line_numbers |= config.hide_original_line_numbers` (`multi_diff_editor.rs:922`); a `bool` can't say "unset".
- **Repro:** `noCards` variant with `hide_original_line_numbers: false`: still hidden.

#### E.24 `setPaddingBottom` and `onDidChangeActiveControl` are missing (P3)
- **Monaco:** `setPaddingBottom(px)` (`multiDiffEditorWidget.ts:212`, `multiDiffEditorWidgetImpl.ts:88-90`, with its own test); `onDidChangeActiveControl`/`getActiveControl` (`multiDiffEditorWidget.ts:218-222`).
- **Port:** no bottom padding; `MultiDiffEditorEvent` has only `CollapsedChanged` (`multi_diff_editor.rs:253-262`), and `active_item()` can only be polled.

## F. Options, themes and languages

Checked and matching: all ~180 option registrations and nested option classes (`common/config/editorOptions.ts:6030-6989` against `options.rs`), validation in `EditorOptions::validated()`, a reader for every option, every `diffEditorDefaultOptions` key, all 179 editor-side `registerColor` calls with their four theme defaults, every built-in theme's token rules and colors, the `TokenTheme` trie, `defineTheme`/`setTheme`, `autoDetectHighContrast`, language registration and guessing, `setLanguageConfiguration` layering, `onLanguage`/`onLanguageEncountered`, every `register*Provider` and its less-used fields, and provider ordering.

Dropped: `allowVariableFonts`/`allowVariableFontsInAccessibilityMode` (#233 tracks them). The reason `book/src/options.md:184` gives, that only injected text carries its own font, is out of date: decorations now set `font_family`, `font_weight` and `font_style` (`decorations.rs:426-432`), and Monaco filters those out in accessibility mode (`filterFontDecorations`, `common/config/editorOptions.ts:3995-4003`, `common/viewModel/viewModelImpl.ts:501,575-580`). That filtering doesn't need GPUI font variations and could be split out of #233.

### Language configuration (#326)

#### F.1 A language with no grammar or tokenizer gets no configuration (P1)
- **Monaco:** configuration doesn't depend on a tokenizer: `register({id})` plus `setLanguageConfiguration(id, {brackets, autoClosingPairs, comments})` gives auto-closing and surround (`common/languages/supports/characterPair.ts:22-37`), bracket matching and Toggle Line Comment.
- **Port:** `TextModel::ensure_syntax` builds `self.syntax` only for `language.filter(|l| l.has_highlighter())` (`model.rs:773`), and `TextModel::language()` reads it (`model.rs:712-714`), so a language without a grammar is `None`. Typing runs with `lang = None` (`editor.rs:3476-3491` → `cursor/language_typing.rs:73-76`); bracket matching returns `None` (`bracket_matching.rs:31`); `language_at` falls back to `None`, so comments find no tokens (`editor.rs:3784-3787`, `model.rs:905-907`); word pattern, folding markers and section headers are lost (`occurrences.rs:59`, `editor.rs:3525`, `section_headers.rs:211`). `set_language_configuration` → `refresh_language` (`model.rs:789-797`) re-runs the filter. The same gap holds while a `register_tokens_provider_factory` task is pending. `book/src/languages.md:8` says the grammar is optional.
- **Repro:** register `Language::new("mylang","My")` with `brackets: [["(",")"]]` and `comments.lineComment: "//"`, no grammar; open a model. `(` inserts no `)`, the bracket isn't matched, and Cmd+/ does nothing.

#### F.2 Plain text, and models without a language, have none of Monaco's plaintext configuration (P1)
Also found by the editing audit (A2).
- **Monaco:** `plaintext` is registered with `.txt`, aliases `Plain Text`/`text` and `text/plain` (`common/languages/modesRegistry.ts:57-62`) and a built-in configuration: brackets `() [] {}`, surrounding pairs ``{} [] () <> "" '' ` ` ``, `colorizedBracketPairs: []`, `folding.offSide` (`common/languages/languageConfigurationRegistry.ts:300-323`). Without `autoClosingPairs` the brackets auto-close (`characterPair.ts:20-26`), and they drive Enter between `{|}`. A model without a language, or with an unknown one, is `plaintext` (`common/services/languageService.ts:124-130`).
- **Port:** `plain_text()` has an empty configuration, no `text/plain` and an extra `txt` alias (`crates/gpui-monaco-languages/src/lib.rs:327-332`); F.1 would drop a configuration anyway. A model with no language passes `None` (`editor.rs:3462-3492`). The unit harness defines Monaco's plaintext configuration itself (`cursor/tests/language_configs.rs:100-115`), so tests pass while the shipped editor differs.
- **Repro:** `CodeEditor::with_text("", …)` with no language. `(` gives `(` (Monaco `()`); select `foo`, type `"`: `"` (Monaco `"foo"`); Enter in `{|}`: a plain break (Monaco indents the middle line); no bracket match highlight.

#### F.3 Registering a language later doesn't re-resolve models that use or would guess it (P2)
- **Monaco:** a model's language is a `LanguageSelection` re-evaluated when registrations change (`common/services/languageService.ts:104-121`, `:155-169`); a model created with `'foo'` before `register({id:'foo'})` is plaintext, then `foo`.
- **Port:** `ensure_syntax` resolves once (`model.rs:738-781`), `guess_language` is `mem::take`n (`:745`), and `LanguageRegistry::register` (`crates/syntax/src/language.rs:462`) notifies nothing. `language_id()` reports the unregistered id rather than `plaintext`, and `on_language` fires for it (`languages.rs:332-345`).
- **Repro:** `.with_language("mylang")`, show it, then register `mylang` with a grammar. The model never highlights.

#### F.4 MIME type lookups: no automatic `text/x-<id>`, and the first registration wins (P3)
- **Monaco:** each language without `mimetypes` gets `text/x-${id}` (`common/services/languagesRegistry.ts:185-190`); the mime map is rebuilt with the last registration winning (`:133-146`).
- **Port:** `LanguageRegistry::language_for_mime_type` uses `find` and adds no `text/x-<id>` (`crates/syntax/src/language.rs:638-645`).
- **Repro:** `model.set_language(Some("text/x-rust".into()))` leaves the model without a language.

#### F.5 Standard token types use prefix checks instead of Monaco's word-boundary regex (P3)
- **Monaco:** `/\b(comment|string|regex|regexp)\b/` anywhere, first match wins (`common/languages/supports/tokenization.ts:248-265`).
- **Port:** `token_type_at` uses `starts_with("comment")`, `contains("regex")`, `starts_with("string")`, and treats `character` as a string (`model.rs:948-959`). `string.regexp` is a String in Monaco and a RegEx here, which changes `autoClosingPairs.notIn` and quick suggestions.
- **Repro:** a Monarch grammar emitting `invalid.string` or `meta.comment`: quotes still auto-close inside, and `quick_suggestions.strings: Off` doesn't apply.

#### F.6 A tokens provider factory replaces a tokenizer already set (P3)
- **Monaco:** `TokenizationRegistry.getOrCreate` returns an existing support and never resolves the factory (`common/tokenizationRegistry.ts:63-79`).
- **Port:** `register_tokens_provider_factory` runs at first request regardless and installs over the current highlighter (`languages.rs:178-216`).
- **Repro:** `set_monarch_tokens_provider(cx, "x", …)`, then `register_tokens_provider_factory(cx, "x", …)`, open a model in `x`: the factory's tokenizer wins.

### Theme colors resolved but never drawn (#328, #342)

#### F.7 Hint markers draw nothing; Monaco draws three dots (P2)
- **Monaco:** untagged `Hint` markers get `squiggly-hint` (`common/services/markerDecorationsService.ts:235-243`): a three-dot SVG in `editorHint.foreground` under the first two characters (`browser/widget/codeEditor/codeEditorWidget.ts:2554-2557`), plus a `2px dotted` `editorHint.border` in high contrast (`browser/widget/codeEditor/editor.css:111-113`).
- **Port:** `markers.rs:240-245` gives hints no underline, and the element draws markers only through `options.underline` (`element.rs:4054-4085`). `editor_hint_foreground`/`border` (`theme.rs:690-692`) are never read for hints, though `markers.rs:224-230` shortens the range "to give them room for their three dots". `book/src/diagnostics.md:27` says hints get no squiggle.
- **Repro:** `set_model_markers` with an untagged `MarkerSeverity::Hint`: no mark.

#### F.8 High-contrast themes don't draw the error, warning and info double underline (P2)
- **Monaco:** `.squiggly-error/-warning/-info { border-bottom: 4px double var(--vscode-editor{Error,Warning,Info}-border) }` (`browser/widget/codeEditor/editor.css:81-109`), set in hc-black and hc-light.
- **Port:** `editor_error_border` and friends are resolved (`theme.rs:675-687`, `theme/color_registry.rs:305-311`) but read only by a test (`theme.rs:2944`); the element draws only the wavy underline (`element.rs:4054-4085`).
- **Repro:** `set_theme(cx, "hc-black")` with an error marker.

#### F.9 `editorBracketMatch.foreground` is never applied (P2)
- **Monaco:** matched brackets carry `inlineClassName: 'bracket-match-inline'` (`contrib/bracketMatching/browser/bracketMatching.ts:304,315`), colored by `editorBracketMatch.foreground` over pair colorization (`:421-428`).
- **Port:** `bracket_match_foreground` is resolved (`theme.rs:600`) but unread; the decoration sets only background, border and ruler (`bracket_matching.rs:129-140`), though `DecorationOptions.text` (`decorations.rs:337`) could carry it.
- **Repro:** define a theme with `"editorBracketMatch.foreground": "#ff0000"`: matched brackets keep their pair color.

Other resolved fields with no reader, for the record (dead weight rather than bugs): `button_separator`, `message_background`/`message_foreground`, `input_validation_warning_*`, `checkbox_*`, `icon_foreground`, `list_active_selection_foreground`, `list_active_selection_icon_foreground`, `list_focus_foreground`, `list_focus_highlight_foreground`, `list_invalid_item_foreground`, `list_deemphasized_foreground` and `quick_input_list_focus_icon_foreground`.

### Options and theme defaults (#348)

#### F.10 `occurrencesHighlight: MultiFile` acts like `SingleFile` (P3)
Documented in `book/src/options.md:123` and `MONACO_PARITY.md:100`, but the reason ("no multi-document highlight providers and editors don't share highlights") doesn't hold.
- **Monaco:** with `'multiFile'`, `WordHighlighter._run` also highlights the models of every other open editor (`contrib/wordHighlighter/browser/wordHighlighter.ts:612-680`), and standalone registers a textual multi-document provider for `'*'` (`textualHighlightProvider.ts:81-87`, `wordHighlighter.ts:1024`). The port can list editors (`registry::editors`).
- **Port:** `options.rs:158-162`.
- **Repro:** two editors, same language, `MultiFile`, cursor on a word in one: the other shows nothing.

#### F.11 An invalid `minimap.markSectionHeaderRegex` turns MARK headers off (P3)
- **Monaco:** `EditorMinimap.validate` keeps the default if `new RegExp(input, 'd')` throws (`common/config/editorOptions.ts:3597-3604`).
- **Port:** stored as given; `collect_mark_headers` returns nothing when `Pattern::new` fails (`section_headers.rs:113-117`); `validated()` doesn't check it (`options.rs:833-840`).
- **Repro:** `mark_section_header_regex: "("`: every `// MARK:` header disappears.

#### F.12 The default theme is vs-dark; Monaco's is vs (P3)
- **Monaco:** `setTheme(VS_LIGHT_THEME_NAME)` at start (`standalone/browser/standaloneThemeService.ts:261`).
- **Port:** `global_theme(cx).unwrap_or_else(|| Arc::new(Theme::vs_dark()))` (`editor.rs:843`); `EditorColors::default()`, the multi diff editor (`multi_diff_editor.rs:2253-2255`) and high-contrast switching (`high_contrast.rs:71`) do the same. `book/src/themes.md:22` calls vs-dark the default; `differences.md` doesn't list it.
- **Fix:** switch to `vs`, or document it in `differences.md`.

#### F.13 Invalid theme color values are dropped instead of becoming red (P3)
- **Monaco:** `Color.fromHex` returns `Color.red` for an unparseable value (`base/common/color.ts:258-260`), for `defineTheme` colors (`standaloneThemeService.ts:80`) and VS Code themes.
- **Port:** `parse_colors` drops them (`theme.rs:1248-1253`).
- **Repro:** `{"editor.background": "#12345"}` keeps the base background; Monaco paints red. If deliberate, document it.

## G. Public API

Scope: `monaco.d.ts` (`monaco.editor.*`, `IEditor`, `ICodeEditor`, `IDiffEditor`, `ITextModel`, `IModelDecorationOptions`, `IActionDescriptor`, event payloads), `editor.api.ts`, `standalone/browser/standaloneEditor.ts`, `standaloneCodeEditor.ts`, `browser/editorBrowser.ts`, `common/textModelEvents.ts`, `standalone/browser/colorizer.ts` and `common/services/markerDecorationsService.ts`, against `lib.rs`, `api.rs`, `editor_api.rs`, `editor.rs`, `model.rs`, `events.rs`, `decorations.rs`, `registry.rs`, `commands.rs`, `openers.rs`, `colorize.rs`, `markers.rs`, `editor_actions.rs`, `editor_commands.rs`, the `diff_editor.rs` API and `book/src/migrating.md`. The decoration hover findings from this audit are in §C.6, §C.8 and §C.9.

Checked and matching: the event payloads, global registries and events, `getModelMarkers` filters, commands and actions registration, every `reveal*` and geometry getter, `executeEdits` with `forceMoveMarkers` and cursor state computers, `executeCommand(s)`, undo stops, decoration getters and collections, EOL and UTF-16 offsets, and the diff editor's `getLineChanges`, `waitForDiff` and `setModel(null)`.

### Cursor API (#323, #349, #355)

#### G.1 `set_position`, `set_selection` and `set_selections` scroll the cursor into view (P1)
Grouped with the navigation audit's reveal findings (§D.16–§D.19); same root cause as §D.16 and §D.17.
- **Monaco:** `setSelections(ranges, source = 'api')` → `viewModel.setSelections` (`browser/widget/codeEditor/codeEditorWidget.ts:949-961`) → `cursor.setStates` (`common/cursor/cursor.ts:122-138`, `313-314`); `_emitStateChangedIfNecessary` (`:411-434`) only emits events. Hosts reveal themselves (`setPosition(p)` then `revealPositionInCenter(p)`).
- **Port:** `set_selections` (`editor.rs:1272-1293`, used by `set_position`/`set_selection` at `api.rs:1763-1776`) calls `cursors_changed(true)`, which sets `autoscroll_requested` whatever the source when the state changed (`editor.rs:2699-2705`); only the unchanged path checks `source != Api` (`:2687`). The next frame scrolls vertically (`element.rs:3082-3100`) and horizontally (`:3359-3373`).
- **Repro:** restore a scroll position, then set an off-screen cursor: the editor jumps to it. Syncing selections into a background editor scrolls it. `set_position` after `reveal_*` reveals twice.
- **Fix:** no autoscroll for `CursorChangeSource::Api` in `set_selections`; commands already pass a non-API source.

#### G.2 Cursor-change `source` is a closed enum, so hosts can't tag their own changes (P2)
- **Monaco:** `setPosition`, `setSelection(s)` (`codeEditorWidget.ts:949`), `executeEdits(source, …)`, `executeCommand(s)(source, …)` and `trigger(source, …)` take a free-form `source` returned verbatim in `ICursorSelectionChangedEvent.source` and `ICursorPositionChangedEvent.source` (`cursor.ts:431`). The usual pattern: `executeEdits('my-ext', …)` and ignore `e.source === 'my-ext'`.
- **Port:** `CursorChangeSource` is Keyboard, Mouse, Api, Model, ModelChange or RestoreState (`events.rs:232`). The setters and `execute_edit(_operation)s` always report `Api` (`editor.rs:1288`, `api.rs:1091`, `api.rs:1124`); `execute_command(s)` maps everything but `"keyboard"`/`"mouse"` to `Api` (`editor_commands.rs:1195-1200`); `trigger` takes no source.
- **Fix:** a `CursorChangeSource::Other(SharedString)` variant and optional `source` parameters (or `*_from` variants).

#### G.3 During a smooth scroll, `scroll_top()` and `ScrollChanged` report the destination (P3)
- **Monaco:** `getScrollTop`/`getScrollLeft` return the animated position (`codeEditorWidget.ts:977-1002`); `getFutureScrollPosition` is the destination; `onDidScrollChange` fires each step.
- **Port:** `scroll_top()` is "where the scroll is going" (`geometry.rs:169-173`); `set_scroll_position_with_scroll_type` emits one `ScrollChanged` with the target (`editor.rs:1351-1363`, `1369-1395`).
- **Repro:** `smooth_scrolling` on; a host mirroring the scroll (a linked editor) jumps instead of following.

### Decorations API (#349)

#### G.4 Edits never fire `DecorationsChanged` (P2)
- **Monaco:** `_doApplyEdits` fires `_onDidChangeDecorations` for every change (`common/model/textModel.ts:1530`), flagging minimap, overview ruler and glyph margin (`:2665-2670`), so `onDidChangeDecorations`, `onDidChangeModelDecorations` and `IEditorDecorationsCollection.onDidChange` fire while decorations move with typing.
- **Port:** `TextModelEvent::DecorationsChanged` comes only from `set_value` (`model.rs:1087`), `delta_decorations` (`:1203`), `remove_all_decorations_with_owner` (`:1221`) and `change_decoration_options` (`:1258`).
- **Repro:** a breakpoint list built from `collection.ranges()` on `DecorationsCollection::on_did_change` goes stale while typing.

#### G.5 No-op decoration deltas still fire `DecorationsChanged` (P3)
- **Monaco:** `deltaDecorations([], [])` returns early (`textModel.ts:1722-1725`); otherwise it fires only when a node is removed or added (`checkAffectedAndFire`, `:1976-2060`).
- **Port:** `delta_decorations` emits unconditionally (`model.rs:1176-1206`), as do `delta_decorations_at`, `CodeEditor::delta_decorations` and `remove_decorations` with stale ids.
- **Repro:** a listener that re-applies decorations on `ModelDecorationsChanged` loops.

#### G.6 Smaller gaps in decoration options and getters (P3)
- **Options:** `linesDecorationsTooltip` (`monaco.d.ts:1776`), `textDirection` (`:1818`), `blockIsAfterEnd`/`blockDoesNotCollapse` (`:1703-1704`; `BlockDecoration` at `decorations.rs:476-484` has only a border and padding).
- **Getters:** `getCustomLineHeightsDecorations(InRange)` (`ITextModel`), and the `filterFontDecorations`, `onlyMinimapDecorations` and `onlyMarginDecorations` filters (`api.rs:1500-1660`).

### Options API (#348)

#### G.7 Default options are VS Code's core defaults, not standalone Monaco's (P2)
- **Monaco:** `editor.api.ts:14-17` overrides `wrappingIndent: None`, `glyphMargin: false`, `autoIndent: Advanced` and `overviewRulerLanes: 2`.
- **Port:** `glyph_margin: true` (`options.rs:2139`), `overview_ruler_lanes: 3` (`:2177`), `wrapping_indent: Same` (`:2196`), `auto_indent: Full` (`:2224`, `cursor/mod.rs:169-170`). Audit 2 (`MONACO_AUDIT_2.md:2013`) checked these against `editorOptions.ts` only.
- **Repro:** a fresh editor has a glyph margin, three ruler lanes, indented wrapped lines and full reindenting.
- **Fix:** change the defaults, or document the choice in `options.md` and `differences.md`.

#### G.8 There's no `getOption`/`getOptions` for computed options, and `migrating.md` maps `getOptions` to the raw options (P2)
- **Monaco:** `getOptions()`/`getOption(EditorOption.x)` return computed values (zoomed `fontSize`, resolved `lineHeight`, `accessibilitySupport` from `auto`, effective `wordWrap`, `readOnly`, `tabFocusMode`, `layoutInfo`, `fontInfo`); `getRawOptions()` returns what was set (`monaco.d.ts:6229-6237`).
- **Port:** `CodeEditor::options()` returns `user_options` (`getRawOptions`, `editor.rs:1174-1183`); the validated, zoomed options are private apart from `font_info()` and `layout_info()`. `ConfigurationChange::new_options()` (`events.rs:295-325`) is the computed set, so under zoom it disagrees with `options()`. `book/src/migrating.md:65` and `:78` map `getOptions()` to `options()`, which `book/src/options.md:145` calls `getRawOptions`.
- **Fix:** add `computed_options()` and fix the rows.

### Markers, models, links and colorize (#326, #342, #353)

#### G.9 Markdown links bypass `register_link_opener` (P2)
- **Monaco:** `registerLinkOpener` registers on `IOpenerService` (`standalone/browser/standaloneEditor.ts:444-454`), through which markdown links open (`openLinkFromMarkdown`, `contrib/message/browser/messageController.ts:74`, and the hover renderer), so a host sees every clicked link.
- **Port:** `markdown.rs` `open_link` (`:818-827`) opens http(s) and mailto (`is_openable`, `:761-766`) and runs trusted `command:` links; other schemes are dropped and openers never asked. Only detected links (`links.rs:387`) and marker code links (`markers.rs:112`) use `open_link_with_openers`.
- **Repro:** a hover with a `file://` or custom-scheme link: a registered opener isn't called.

#### G.10 Models without a uri are invisible to the global marker events and filters (P2)
- **Monaco:** a model without a uri gets `inmemory://model/N` (`common/model/textModel.ts:318-322`), so `setModelMarkers` fires `onDidChangeMarkers([uri])` and `getModelMarkers({resource})` finds them.
- **Port:** `TextModel::uri()` is `Option` (`model.rs:533-536`) and `create_model(cx, value, lang, None)` leaves it `None` (`:117-140`); `set_markers` calls `registry::markers_changed` only with a uri (`markers.rs:277-279`); `ModelMarker.resource` is `None`.
- **Repro:** set markers on a uri-less model: `on_did_change_markers` never fires and `MarkerFilter { resource }` can't select them.
- **Fix:** generate `inmemory://model/N`, at least in `create_model`.

#### G.11 Markers die with their model, and there's no 500-marker cap (P3)
- **Monaco:** markers live in `IMarkerService` by uri; disposal removes only `inmemory:`, `internal:` and `vscode:` ones (`common/services/markerDecorationsService.ts:99-112`); a new model with the same uri is redecorated (`:93-97`); decorations are capped at 500 markers (`:114-116`).
- **Port:** markers are stored on the `TextModel` (`markers.rs:200-281`) and vanish with it (`registry.rs:149-163`); `model_markers` walks live models (`markers.rs:374-404`); every marker is decorated.
- **Repro:** set markers on a `file://` model, drop it, create it again: no markers.

#### G.12 Empty markers aren't widened to the word, and full-line markers aren't trimmed (P3)
- **Monaco** (`_createDecorationRange`, `markerDecorationsService.ts:190-222`): an empty marker becomes the word at its position (or stays empty, drawn 1ch wide, on an empty line or past the last non-whitespace); a `startColumn 1 … endColumn MAX` single-line marker starts at the first non-whitespace character.
- **Port:** widens by one character (`markers.rs:234-239`); at a line end that is the `\n`, so the squiggle spans the break. No word widening or leading-whitespace trim, though the comment says "to the word or character".
- **Repro:** an empty marker in the middle of `foobar`, and one at the end of a line.

#### G.13 `colorize`/`colorize_element` don't accept a MIME type (P3)
- **Monaco:** `colorizeElement` resolves `getLanguageIdByMimeType(mimeType) || mimeType` (`standalone/browser/colorizer.ts:33-38`).
- **Port:** `find_language` looks up by id, then name (`colorize.rs:60-65`).
- **Repro:** `colorize(…, "text/x-rust")` finds nothing; models already accept MIME types.

#### G.14 Model uris are compared as raw strings (P3)
- **Monaco:** `getModel(uri)` and the marker service key on normalized `URI.toString()`.
- **Port:** `model_for_uri`/`live_model_for_uri` and `MarkerFilter.resource` compare strings as given (`model.rs:89-109`, `markers.rs:380-385`).
- **Repro:** `file:///a%20b` vs `file:///a b`, or a different drive-letter case, don't match.

### Remaining API gaps (#349, #353)

#### G.15 No `getId`, `getEditorType`, `EditorType` or `ITextModel.id` (P3)
- **Monaco:** `IEditor.getId()`/`getEditorType()` (`monaco.d.ts:2686-2691`), `editor.EditorType`, `ITextModel.id` (`:2034`; `'$model' + N`, `common/model/textModel.ts:319`).
- **Port:** none; `entity_id()` serves as the id.
- **Fix:** document the mapping in `migrating.md`.

#### G.16 `IActionDescriptor`: no `keybindingContext`, no run arguments, no `getAction(id)` object (P3)
- **Monaco:** `IActionDescriptor { …, keybindingContext?, run(editor, ...args) }` (`monaco.d.ts:1241-1260`); `getAction(id)` returns an `IEditorAction` with `id`, `label`, `alias`, `metadata`, `isSupported()` and `run(args)`.
- **Port:** `EditorActionDescriptor` (`editor_actions.rs:27-47`) lacks the context and arguments; `run_command` skips host actions when a rule has `command_args` (`:1097`); `trigger_with_payload` drops the payload for added actions (`api.rs:1184-1196`); only `run_action(id)` and `supported_actions()` exist.

#### G.17 Language-change events have no `source` (P3)
- **Monaco:** `IModelLanguageChangedEvent.source` (`common/textModelEvents.ts:30`).
- **Port:** `TextModelEvent::LanguageChanged` (`model.rs:343-348`) and `CodeEditorEvent::ModelLanguageChanged` (`editor.rs:126`).

#### G.18 Editor events still missing (P3)
- **Monaco** (`browser/editorBrowser.ts`): `onDidChangeLineHeight` (`:932`), `onDidChangeFont` (`:939`), `onDropIntoEditor` (`:783`), `onWillTriggerEditorOperationEvent` (`:841`) and `onMouseLeave`'s `IPartialEditorMouseEvent` (`:798`).
- **Port:** `CodeEditorEvent` (`editor.rs:45-205`) has none, and `MouseLeave` has no payload. Audit 2 §I.17 listed the first three; `MONACO_PARITY.md:135` marks the area done.

#### G.19 Model-level `apply_edits`/`push_edit_operations` drop `forceMoveMarkers` (P3)
- **Monaco:** both take `IIdentifiedSingleEditOperation[]` with `forceMoveMarkers`.
- **Port:** both take `Vec<(Range<PointUtf16>, String)>` (`api.rs:651-713`); only `CodeEditor::execute_edit_operations` takes `SingleEditOperation`.

#### G.20 `TextModel::validate_range` shrinks a range whose end splits a surrogate pair (P3)
- **Monaco:** `validateRange` expands such a range (start − 1, end + 1) and moves only collapsed ranges left (`common/model/textModel.ts:1110-1160`).
- **Port:** both ends clip with `Bias::Left` (`api.rs:63-71`); `Rope::clip_range_utf16` already does Monaco's version (`crates/text/src/rope.rs:439-448`). Affects `value_in_range`, `delta_decorations_at`, collections, `find_matches_in` and `apply_edits`.
- **Repro:** `value_in_range` over `a😀b` ending inside the emoji.

#### G.21 `word_at_position` ignores embedded languages and token boundaries (P3)
Found by the API audit (E7) and the editing audit (D8).
- **Monaco:** `getWordAtPosition` uses the word definition of the language at the position's token, limited to that language's run, with a left-biased retry at a boundary (`common/model/tokens/tokenizationTextModelPart.ts:265-310`).
- **Port:** the model's top-level `word_pattern` over the whole line (`api.rs:174-190`), though `language_at` exists (`model.rs:905`).
- **Repro:** HTML with an embedded `<script>` whose language has a different `wordPattern`; a word touching the boundary differs.

#### G.22 Small `ITextModel` helpers are missing (P3)
- `isValidRange` (`monaco.d.ts:2155`), `getLineMinColumn` (`:2118`; `getLineMaxColumn` is only `line_len_utf16() + 1`), and `getLanguageIdAtPosition` (not in `migrating.md`).

### Docs (#348)

#### G.23 `migrating.md` rows that are wrong or missing (P3)
- `book/src/migrating.md:21`: `setModelLanguage` → `model.set_language(id, cx)`; it takes `Option<SharedString>`.
- `:32`: `add_keybinding_rules` takes a `Vec`, and both return `anyhow::Result<Disposable>` (`editor_actions.rs:835-871`).
- `:79`: `setPosition`/`setSelection` don't mention G.1 or G.2. (`:65` and `:78` are G.8.)
- No rows for `getRawOptions`, `getOption`, `getId`/`getEditorType`, `getAction(id)` or `getLineHeightForPosition` (`line_height_for_position`, `geometry.rs:88-98`).

#### G.24 The `content_height` docs contradict the code (P3)
- `editor.rs:1410-1416` and `book/src/model-and-view.md:139` say `content_height` leaves out the horizontal scrollbar's height. `element.rs:2956-2963` adds `hbar_height` unless `ignore_horizontal_scrollbar_in_content_height`, matching Monaco's `ViewLayout._getContentHeight` (`common/viewLayout/viewLayout.ts:263-274`) and `book/src/options.md:69`. Only the `scroll_beyond_last_line` room is left out, as `MONACO_PARITY.md:141` says.

## H. Input, accessibility and keybindings

`quickInputActions.ts`, `aria.ts` and `scrollableElement.ts` aren't vendored; findings that depend on them say so.

**How GPUI resolves one key in one context.** `Keymap::resolve_bindings_for_input` (gpui-pre 0.3.8 `src/keymap.rs:216-218`) sorts matches by context depth, then by binding index with later bindings first, and `window.rs:6014-6026` runs the first unless it calls `cx.propagate()`. A predicate like `CodeEditor && x` matches at the same depth as plain `CodeEditor`, and the platform blocks are appended after the main list in `default_key_bindings`, so a plain platform binding beats a contextual binding in the main list with the same keys. This causes H.1 and H.2; moving contextual bindings after the platform blocks, plus a test that checks every contextual binding against plain bindings on the same keys, would fix both.

Checked and matching: every `kbOpts` and `KeybindingsRegistry` rule in `browser/coreCommands.ts`, `browser/editorExtensions.ts`, `contrib/**`, `standalone/browser/quickAccess/*` and `browser/widget/diffEditor/commands.ts`, with their platform overrides and secondaries (`actions.rs:1676-2282`, `diff_editor.rs:98-134`); folding preconditions; click counting, Shift-click, the macOS Ctrl+click menu, gutter focus, `suppressMouseDown`, middle-click column select and autoscroll, the Linux primary selection, drag and drop, wheel zoom modifiers, `fastScrollSensitivity`, `scrollPredominantAxis`, `alwaysConsumeMouseWheel`; `emptySelectionClipboard`, `multicursorText`, `multiCursorPaste`, composition at every cursor while marked, `tabFocusMode` and its keys, `accessibilityPageSize`; the editor's aria label, role and active descendant, screen-reader wrapping, the accessible diff viewer, and the existing announcements. Standalone Monaco has no Alt+F1 help and no Insert overtype binding in this version, so neither is a gap.

### Keybindings (#332, #336)

#### H.1 Cmd/Ctrl+Right never accepts the next word of ghost text (P1)
- **Monaco:** `editor.action.inlineSuggest.acceptNextWord` binds CtrlCmd+Right on every platform with weight `EditorContrib + 1`, above `cursorWordEndRight` and `cursorEnd`, and requires `!accessibilityModeEnabled` (`contrib/inlineCompletions/browser/controller/commands.ts:141-147`).
- **Port:** `AcceptNextInlineWord` is in the main list (`actions.rs:1845-1853`); the later platform bindings `cmd-right` → `CursorEnd` (`:1937`) and `ctrl-right` → `CursorWordRight` (`:2005`) win, and `move_cursors` never propagates (`editor.rs:3080-3170`, `:6166-6171`). The binding also lacks `!accessibilityModeEnabled`, so once reordered it would take the key from screen reader users.
- **Repro:** ghost text showing, cursor before it, Cmd+Right (macOS) or Ctrl+Right. The cursor moves; no word is accepted. No test covers this key.

#### H.2 Quick input: Cmd/Ctrl+Home/End and, on macOS, Ctrl+N/P never reach the list (P2)
- **Monaco:** `quickInputActions.ts` (upstream, not vendored) binds `quickInput.first`/`last` and macOS `quickInput.next`/`previous` (WinCtrl+N/P). VS Code uses Ctrl rather than Cmd for first/last on macOS; check that too.
- **Port:** `QUICK_INPUT` binds `secondary-home`/`secondary-end` → `QuickInputFirst`/`Last` and `ctrl-n`/`ctrl-p` → `QuickInputNext`/`Previous` (`actions.rs:1832-1840`, "the list's keys win over the input's"), but plain `CodeEditor` bindings registered later win: macOS `cmd-home`/`cmd-end` (`:1943-1944`) and `ctrl-n`/`ctrl-p` (`:1916-1917`), Windows/Linux `ctrl-home`/`ctrl-end` (`:2009-2012`). The quick input's field is a `CodeEditor` (`quick_access.rs:769-790`).
- **Repro:** F1, then Cmd+End (macOS) or Ctrl+End: the focused item doesn't move. On macOS, Ctrl+N does nothing.

#### H.3 Tab focus mode takes Tab from the suggest widget and snippets (P2)
- **Monaco:** only `tab`, `outdent` (`browser/coreCommands.ts:2008-2012`, `2031-2035`) and `inlineSuggest.commit` (`commands.ts:209-221`) require `!editorTabMovesFocus`. `acceptSelectedSuggestion` (weight +90, `suggestWidgetVisible && textInputFocus`, `contrib/suggest/browser/suggestController.ts:843-851`), `jumpToNext/PrevSnippetPlaceholder` (+30, `contrib/snippet/browser/snippetController2.ts:349-366`) and `insertNext/PrevSuggestion` (`suggestController.ts:1085-1114`) don't.
- **Port:** `tab` → `FocusNextElement` in `TAB_MOVES_FOCUS` is bound after `Tab` "so they win" (`actions.rs:1802-1804`), and suggest acceptance and snippet jumps live inside the `Tab` handler (`editor.rs:3624-3663`).
- **Repro:** Ctrl+M (Ctrl+Shift+M on macOS), Ctrl+Space, Tab. Focus leaves the editor and the suggestion isn't accepted. Same in a snippet with a next tab stop.

#### H.4 Tab and Shift+Tab are trapped in read-only editors (P2, accessibility)
- **Monaco:** `tab` and `outdent` have `precondition: EditorContextKeys.writable` (`browser/coreCommands.ts:2005`, `2028`), so in a read-only editor (including the diff editor's original side) the browser moves focus on.
- **Port:** no precondition (`editor_actions.rs:439-440`); `CodeEditor::tab` → `insert_tab` → `apply_inner` shows the read-only message and keeps focus (`editor.rs:3661`, `:2553-2557`); `outdent` likewise through `run_edit`.
- **Repro:** `read_only: true`, Tab: "Cannot edit in read-only editor", and keyboard users can't leave without tab focus mode.

#### H.5 Shift+Tab in a snippet choice accepts the alternative suggestion instead of going back (P3)
- **Monaco:** a second `jumpToPrevSnippetPlaceholder` rule with weight `EditorContrib + 91` for `inSnippetChoice` beats `acceptAlternativeSelectedSuggestion` (+90) (`snippetController2.ts:373-378`).
- **Port:** `shift-tab` → `AcceptAlternativeSuggestion` in `SUGGEST` (`actions.rs:1864`) wins; `accept_alternative_suggestion` doesn't check snippet choices (`completion.rs:2095-2097`).
- **Repro:** snippet `${1:a} ${2|x,y|}`, Tab to the choice, Shift+Tab. The port accepts `x` or `y`; Monaco returns to `$1`.

#### H.6 Escape order differs from Monaco's weights (P3)
`CodeEditor::escape` hard-codes an order (`editor.rs:3341-3418`) that differs in three places:
- **Snippet vs. problem zone:** Monaco closes the zone first (`closeMarkersNavigation` +50, `contrib/gotoError/browser/gotoError.ts:299`; the snippet is +30).
- **Peek:** in the host editor `closeReferenceSearch` has `WorkbenchContrib + 50` (`contrib/gotoSymbol/browser/peek/referencesController.ts:373-386`), above suggest, linked editing and snippets; the port checks peek after them.
- **Find vs. symbol navigation:** Monaco closes find (+5, `contrib/find/browser/findController.ts:1076`) before `editor.gotoNextSymbolFromResult.cancel` (`contrib/gotoSymbol/browser/symbolNavigation.ts:173-176`); the port the reverse.
- **Repro:** open a peek, type in the host until suggest opens, Escape. Monaco closes the peek; the port closes suggest.

#### H.7 The Menu (Apps) key doesn't open the context menu (P3)
- **Monaco:** `KeyCode.ContextMenu` opens it (`contrib/contextmenu/browser/contextmenu.ts:68-78`).
- **Port:** only Shift+F10 (`actions.rs:1792`); GPUI reports VK_APPS as `"menu"` (gpui-pre-windows 0.3.8 `src/events.rs:1568`), unbound.
- **Repro:** press the Menu key on Windows.

### Mouse and IME (#350, #356)

#### H.8 Cmd/Ctrl+click and middle-click "openLink" act on mouse-down, can't be cancelled, and block Cmd-drag (P2)
- **Monaco:** `ClickLinkGesture` records the modifier on mouse-down and fires on mouse-up, only on the same line (`contrib/gotoSymbol/browser/link/clickLinkGesture.ts:194-209`); a drag or selection change resets it (`:170`, `:182-186`); the ordinary mouse-down still runs (`browser/controller/mouseHandler.ts:330-332`), so Cmd+drag selects.
- **Port:** `on_mouse_down` follows the link or definition at once and returns before any mouse-down operation (Cmd/Ctrl+click `editor.rs:4786-4796`; middle-click `OpenLink` and `CtrlLeftClick` `:4410-4441`).
- **Repro:** Cmd+press on a URL, drag away, release: the port has opened it. Cmd+drag from an identifier with a definition: the port navigates; Monaco selects.

#### H.9 Trackpad pinch doesn't zoom (P3)
- **Monaco:** `hasMouseWheelZoomModifiers` accepts Ctrl on macOS, "the two fingers pinch gesture" (`browser/controller/mouseHandler.ts:190-198`).
- **Port:** GPUI delivers pinch as `PinchEvent` (gpui-pre 0.3.8 `src/interactive.rs:567-585`, `div.rs:405` `on_pinch`); the editor has no `on_pinch` (`font_zoom.rs:84-125`).
- **Repro:** `mouse_wheel_zoom: true`, pinch on a Mac trackpad.

#### H.10 Touch isn't handled, and the docs don't say so (P3, docs)
- **Monaco:** `browser/controller/pointerHandler.ts`: tap places the cursor, pan scrolls, long press opens the menu.
- **Port:** GPUI 0.3.8's `TouchEvent` dispatch is "core implementation pending" (`interactive.rs:111-117`); the editor uses none, and `book/src/differences.md` doesn't mention touch.
- **Fix:** one line in the differences page; revisit when GPUI dispatches touch.

#### H.11 macOS press-and-hold accents replace the character at the primary cursor only (P3)
- **Monaco:** the IME's replacement arrives as `compositionType(text, replacePrevCharCnt, …)` (`browser/controller/editContext/native/nativeEditContext.ts:443-473`) and applies to every selection (`common/cursor/cursor.ts:589-605`).
- **Port:** `replace_text_in_range` with an explicit range and no marked text calls `replace_range` once at the IME's (primary) range (`editor.rs:6021-6046`); the every-cursor path needs marked text (`:6028-6037`).
- **Repro:** two cursors after `e`, hold `e`, pick `é`: only the primary changes.

### Accessibility (#347)

#### H.12 Screen reader content grows with the selection (P2)
- **Monaco:** `SimplePagedScreenReaderStrategy.fromEditorSelection` uses at most two pages: the start page's pretext, the selection, the end page's posttext, with the middle of a long selection replaced by `…` (`browser/controller/editContext/screenReaderUtils.ts:51-110`, `:82-91`). With `accessibilityPageSize` 500 that's about 1,000 lines at most. It reads the view model, so folded lines are left out and wrapped lines are view lines.
- **Port:** `screen_reader_content` takes every row from the start page to the end page (`accessibility.rs:65-79`; the test expects it, `:287-290`), and `build` makes one AccessKit `TextRun` per line every frame while assistive technology is active (`editor.rs:6283-6299`, `accessibility.rs:105-139`). It uses model rows (P3 on its own).
- **Repro:** VoiceOver or NVDA on, Cmd/Ctrl+A in a 100k-line file: every line becomes a node and the frame time jumps.

#### H.13 F8 and Go to Definition are silent to screen readers (P2)
Found by the input audit (A2), the widgets audit (A6) and the navigation audit (4.4).
- **Monaco:** the marker zone's message has `role="alert"`, `aria-live="assertive"`, and `aria.status("{severity}: {message} at line {n}, column {c}.")` on every F8, and the body is focusable with `role="tooltip"` and an `aria-label` (`contrib/gotoError/browser/gotoErrorWidget.ts:61-62, 112-115, 207-226, 313-314`). Navigation commands `alert(references.ariaMessage)`: "Found 3 symbols in 2 files", "No results found" (`contrib/gotoSymbol/browser/goToCommands.ts:128`, `referencesModel.ts:207-217`).
- **Port:** `MarkerZone::render` sets no role or label and `markers.rs` never calls `announce`; `goto.rs` makes no `announce` call.
- **Repro:** VoiceOver on (or the AccessKit inspector): F8 on a problem, then F12 on a symbol. Neither is announced.

#### H.14 Other Monaco announcements the port doesn't make (P3)
Found by the input audit (A2) and, item by item, the widgets audit (B6, C5, C12, D9) and the navigation audit (4.5). The port has 15 `announce` call sites; these Monaco ones have no counterpart:

| Monaco | Message | Port |
|---|---|---|
| `contrib/quickAccess/browser/editorNavigationQuickAccess.ts:155` | "Line {0}, column {1}: {line text}" after go to line or symbol | `quick_access.rs` |
| `contrib/anchorSelect/browser/anchorSelect.ts:61` | "Anchor set at {line}:{col}" | `navigation_history.rs:138` |
| `contrib/suggest/browser/suggestController.ts:500, 591-596` | "Accepting '{0}' made {1} additional edits" | `completion.rs:2154-2245` |
| `contrib/hover/browser/hoverCopyButton.ts:45` | "Copied to clipboard" | `hover.rs` |
| `contrib/rename/browser/renameWidget.ts:752` | "Received {0} rename suggestions" | `rename.rs:253-283` |
| `contrib/codeAction/browser/codeActionController.ts:370-391` | "Context: {word} at line {l} and column {c}" on focusing an action | `code_action.rs:513-529` |

- **Repro:** with a screen reader on, accept an auto-import completion, or run `:40` and accept. Nothing is read.

#### H.15 Every announcement is assertive; Monaco's `status()` messages are polite (P3)
- **Monaco:** `alert` for tab focus, messages, inline suggestions and word highlights; `status` for removed cursors (`browser/coreCommands.ts:1826`), cursor added (`contrib/multicursor/browser/multicursor.ts:37`), suggest "Loading…"/"No suggestions." (`contrib/suggest/browser/suggestWidget.ts:494-504`), problems, quick access and hover copy. In VS Code (`aria.ts`, upstream, not vendored), `status` is a polite live region except on macOS, where it falls back to `alert`.
- **Port:** `build_announcement` always makes `Role::Alert` with `Live::Assertive` (`accessibility.rs:40-50`); `announce` has no politeness parameter (`:183-186`). `book/src/differences.md` documents only `alert`.
- **Repro:** on Windows or Linux, "Loading…" or a cursor-added message cuts off what the screen reader is reading.
- **Fix:** add `announce_status` with a `Live::Polite` region (role `Status`).
