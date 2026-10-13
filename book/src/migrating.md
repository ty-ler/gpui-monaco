# Migrating from Monaco

This page maps Monaco's API to gpui-monaco for people porting a Monaco integration. The shapes follow Rust and GPUI:

- **Models and editors are entities.** Create them with `cx.new`, and read or change them through `read`/`update`.
- **Positions are 0-based.** They're `PointUtf16 { row, column }` with UTF-16 columns, where Monaco uses 1-based `{ lineNumber, column }`. `PointUtf16::from_monaco` and `to_monaco` convert.
- **Ranges are `Range<PointUtf16>`.** Monaco's `IRange`, `Selection` and their helpers become a Rust `start..end` and `Selection` (`anchor`, `head`). Mind the ends when porting `Range` helpers: Monaco's `containsPosition`, `containsRange` and `areIntersectingOrTouching` include the end, but Rust's `Range::contains` excludes it, so `range.contains(&p)` misses a position at the end. Write `range.start <= p && p <= range.end` for `containsPosition`, `outer.start <= inner.start && inner.end <= outer.end` for `containsRange`, and `a.start <= b.end && b.start <= a.end` for `areIntersectingOrTouching`; Monaco's `strictContainsRange` and `areIntersecting` are the strict versions.
- **Events are GPUI events.** Subscribe with `cx.subscribe(&editor, …)` instead of `onDid…` listeners.
- **Async results are `Task`s.** Dropping a task cancels it, so there are no cancellation tokens.

[Differences from Monaco](differences.md) lists behavior that differs.

## Creating editors and models

| Monaco | gpui-monaco |
|---|---|
| `monaco.editor.create(el, { value, language })` | `create_model(cx, value, Some(language), None)?`, then `cx.new(\|cx\| CodeEditor::new(model, window, cx))`. Like the model Monaco's `create` makes, it's registered, so `model_for_uri`, `models(cx)` and word-based suggestions see it; a bare `TextModel::new(value).with_language(language)` isn't until you `register_model` it |
| `monaco.editor.createDiffEditor(el, options)`, `IDiffEditor` | `cx.new(\|cx\| DiffEditor::new(original, modified, window, cx))`, then `update_options`; `setModel` is `set_models`, `getLineChanges` `line_changes()`, `goToDiff` `go_to_diff(next, cx)`, `getOriginalEditor`/`getModifiedEditor` `original_editor()`/`modified_editor()`. See [Diff editor](diff-editor.md) |
| `monaco.editor.createMultiFileDiffEditor(el)` | `cx.new(\|cx\| MultiDiffEditor::new(documents, window, cx))` with a `MultiDiffDocument` per file; `set_documents` replaces them. See [Diff editor](diff-editor.md#many-files) |
| `monaco.editor.createModel(value, language, uri)` | `create_model(cx, value, language, uri)`, which fails like Monaco's for a uri a live model has; or `TextModel::new(value).with_language(language).with_uri(uri)` (a language id or MIME type), then `register_model` so providers can find it |
| `monaco.editor.registerLinkOpener(opener)`, `registerEditorOpener(opener)` | `register_link_opener(cx, \|url, cx\| …)`, `register_editor_opener(cx, \|editor, uri, selection, cx\| …)`, returning a `Disposable`; they answer synchronously. Link openers also get the links clicked in markdown (hovers, suggest details, parameter hints, messages), as Monaco's opener service does |
| `monaco.editor.getModel(uri)` | `model_for_uri(cx, uri)`, comparing normalized uris as Monaco does |
| `URI.parse(s).toString()` | `normalize_uri(s)`; a model's `resource()` is its uri in this form |
| `monaco.editor.getModels()`, `getEditors()`, `getDiffEditors()` | `models(cx)`, `editors(cx)`, `diff_editors(cx)` (the editor's own inputs, such as find's, aren't listed, as Monaco's are DOM inputs) |
| `monaco.editor.setModelLanguage(model, id)` | `model.set_language(Some(id.into()), cx)` (`None` for plain text) |
| `monaco.editor.setModelMarkers(model, owner, markers)` | `model.set_markers(owner, markers, cx)`, or `set_markers_for_uri(cx, owner, uri, markers)` for a uri without a model (Monaco's `IMarkerService.changeOne`) |
| `monaco.editor.getModelMarkers({ owner, resource, take })` | `model_markers(cx, &MarkerFilter { owner, resource, take })` |
| `monaco.editor.removeAllMarkers(owner)` | `remove_all_markers(cx, owner)` |
| `monaco.editor.defineTheme(name, data)` | `define_theme(cx, Theme::define(name, &base, inherit, rules, &colors))`, or a theme from `Theme::from_vscode_json` |
| `monaco.editor.setTheme(name)` | `set_theme(cx, name)` (every editor), or `editor.set_theme(Arc::new(theme), cx)` (one) |
| `monaco.editor.colorize(text, lang, opts)` | `colorize::colorize(text, lang, &theme, &registry, opts)` or `colorize_element` |
| `monaco.editor.tokenize(text, lang)` | `colorize::tokenize(text, lang, &registry)` |
| `monaco.editor.colorizeModelLine(model, line, tabSize)` | `colorize::colorize_model_line(model.read(cx), row, tab_size, &theme, cx)` |
| `monaco.editor.addCommand({ id, run })` | `add_command(cx, id, handler)`, returning a `Disposable` |
| `monaco.editor.addEditorAction(descriptor)` | `add_editor_action(cx, EditorActionDescriptor::new(…))` |
| `monaco.editor.addKeybindingRule(rule)`, `addKeybindingRules(rules)` | `add_keybinding_rule(cx, KeybindingRule::new(keystrokes, command))`, `add_keybinding_rules(cx, vec![…])`, returning an `anyhow::Result<Disposable>` |
| `monaco.languages.register(...)` | `set_language_registry(cx, registry)`, with grammars from `gpui-monaco-languages` |
| `monaco.languages.getLanguages()` | `language_registry(cx).languages()` (`LanguageRegistry::languages`), each a `Language` with its id, name, extensions, file names and MIME types |
| `editor.dispose()` / `model.dispose()` | drop the entity |
| `editor.layout()` | not needed: the editor lays itself out every frame |

## Text model

| Monaco `ITextModel` | `TextModel` |
|---|---|
| `getValue()` / `setValue(v)` | `text()` / `set_value(v, cx)` (removes every decoration, as Monaco's) |
| `getValue(eol, preserveBOM)` | `value(EndOfLinePreference::…, preserve_bom)` |
| `getLineCount()`, `getLineContent(n)`, `getLineLength(n)` | `line_count()`, `line_content(row)`, `line_len_utf16(row)` |
| `getOffsetAt(pos)` / `getPositionAt(offset)` | `get_offset_at(point)` / `get_position_at(offset)` (Monaco's: UTF-16 units, line breaks as the model's line ending); `offset_at` / `position_at` are byte offsets into the stored `\n` text |
| `getValueInRange(range, eol)` | `value_in_range(range, EndOfLinePreference::TextDefined)` |
| `getValueLength(eol, preserveBOM)`, `getValueLengthInRange(range, eol)`, `getCharacterCountInRange(range, eol)` | `value_length_with(eol, preserve_bom)`, `value_length_in_range(range, eol)`, `character_count_in_range(range, eol)` |
| `validatePosition`, `validateRange`, `isValidRange(range)` | `validate_position`, `validate_range`, `is_valid_range(&range)` |
| `getLineMinColumn(n)`, `getLineMaxColumn(n)` | `line_min_column(row)`, `line_max_column(row)` (0-based: `0` and the line's UTF-16 length) |
| `getLanguageIdAtPosition(line, column)` | `language_id_at_position(point)` (`language_at(offset)` returns the `Language`) |
| `getWordAtPosition`, `getWordUntilPosition` | `word_at_position`, `word_until_position`, by the word pattern of the (embedded) language at the position |
| `findMatches(...)` | `find_matches(query, is_regex, match_case, whole_word, …)`, or `find_matches_in(query, scope, …, capture_matches, limit)` |
| `findNextMatch(...)` / `findPreviousMatch(...)` | `find_next_match(...)` / `find_previous_match(...)`, returning a `FindMatch` |
| `getLinesContent()`, `getValueLength()`, `getFullModelRange()` | `lines_content()`, `value_length()`, `full_model_range()` |
| `modifyPosition(pos, offset)` | `modify_position(point, offset)` |
| `applyEdits(edits, true)` | `apply_edits(edits, cx)`, returning the undo edits (also recorded for undo) |
| `applyEdits` with `forceMoveMarkers` | `apply_edit_operations(edits, cx)` with `SingleEditOperation::force_move_markers` |
| `pushEditOperations(before, edits, computer)` | `push_edit_operations(before, edits, computer, cx)` with `SingleEditOperation`s (`(range, text).into()`, or `force_move_markers()`), or `edit(edits, cx)` grouped with `start_transaction`/`end_transaction` |
| `pushStackElement()` / `popStackElement()` | `push_stack_element()` / `pop_stack_element()` |
| `getAlternativeVersionId()` | `alternative_version_id()` |
| `bracketPairs.getBracketPairsInRange`, `matchBracket`, `findEnclosingBrackets` | `bracket_pairs_in_range`, `match_bracket`, `find_enclosing_brackets` |
| `guides.getLinesIndentGuides`, `getActiveIndentGuide` | `lines_indent_guides`, `active_indent_guide` |
| `undo()` / `redo()`, `canUndo()` / `canRedo()` | `undo(source, cx)` / `redo(source, cx)`, `can_undo()` / `can_redo()` |
| `getEOL()` / `setEOL(...)` / `pushEOL(...)` | `line_ending()` / `set_line_ending(...)` / `push_line_ending(...)` (an undo step); both are new versions and content changes (`is_eol_change`) |
| `isAttachedToEditor()`, `onDidChangeAttached` | `is_attached_to_editor()`, `TextModelEvent::AttachedChanged` |
| `normalizeIndentation(s)` | `normalize_indentation(s)` |
| `getLineFirstNonWhitespaceColumn(n)`, `getLineLastNonWhitespaceColumn(n)` | `line_first_non_whitespace_column(row)`, `line_last_non_whitespace_column(row)` (0-based, `None` for blank lines) |
| `tokenization.getLineTokens(n)` | `line_tokens(row, cx)` (`colorize::Token`s) |
| `getOptions()` / `updateOptions(...)` | `options()` / `update_options(...)` (tab size, insert spaces, …) |
| `deltaDecorations(old, new)` | `delta_decorations_at(owner, &old, new, cx)` (positions), or `delta_decorations` with byte ranges |
| `getDecorationsInRange(range, ownerId, filterOutValidation, filterFontDecorations, onlyMinimapDecorations, onlyMarginDecorations)` | `decorations_in_point_range(range, owner, filter)` (positions; `filter` a `DecorationFilter`, or a `bool` for `filterOutValidation`), or `decorations_in_range(range, editor)` with byte ranges |
| `getDecorationRange(id)`, `getDecorationOptions(id)`, `getLineDecorations`, `getLinesDecorations`, `getAllDecorations`, `getOverviewRulerDecorations`, `getInjectedTextDecorations`, `getAllMarginDecorations` | `decoration_range(id)`, `decoration_options(id)`, `line_decorations`, `lines_decorations`, `all_model_decorations`, `overview_ruler_decorations`, `injected_text_decorations`, `all_margin_decorations` (`ModelDecoration`s) |
| `getCustomLineHeightsDecorations(ownerId)`, `getCustomLineHeightsDecorationsInRange(range, ownerId)` | `custom_line_heights_decorations(owner)`, `custom_line_heights_decorations_in_range(range, owner)` |
| `id` | `id()` (`$model1`, …) |
| `setLanguage(languageId, source)` | `set_language(id, cx)` (source `"api"`), `set_language_from(source, id, cx)` |
| `onDidChangeContent` | `TextModelEvent::Edited { changes, version, eol, is_eol_change, … }` |
| `onDidChangeLanguage`, `onDidChangeOptions`, `onDidChangeDecorations` | `LanguageChanged { old_language, new_language, source }`, `OptionsChanged(ModelOptionsChange)`, `DecorationsChanged(DecorationsChange)` (edits fire it too while the model has decorations; a no-op delta doesn't) |
| `onDidChangeLineHeight`, `onDidChangeFont` | `LineHeightChanged(changes)`, `FontChanged(changes)` (`LineHeightChange`, `FontChange`) |

## Editor

| Monaco `ICodeEditor` | `CodeEditor` |
|---|---|
| `getModel()` / `setModel(m)` / `setModel(null)` / `hasModel()` | `get_model()` (or `model()`) / `set_model(m, cx)` / `set_model(None, cx)` (the editor draws nothing and ignores input) / `has_model()` |
| `getValue()` / `setValue(v)` | `value(eol, preserve_bom, cx)` / `set_value(v, cx)` |
| `getOptions()` / `updateOptions(o)` | `computed_options()` (validated, zoomed and resolved) / `update_options_with(\|o\| o.read_only = true, cx)`, or `update_options(full_options, cx)` |
| `getRawOptions()` | `options()` (as set) |
| `getOption(EditorOption.x)` | `computed_options().x`; `font_info()` and `layout_info()` for `fontInfo` and `layoutInfo` (after the first frame) |
| `getPosition()` / `setPosition(p, source)`, `getSelection()` / `setSelection(s, source)` | `position()` / `set_position(p, cx)`, `selection()` / `set_selection(s, cx)`; `set_position_from(source, p, cx)`, `set_selection_from(source, s, cx)` to name the cursor events' `source` (a `CursorChangeSource`, `Other` for a host's own name) |
| `getConfiguredWordAtPosition(p)` | `configured_word_at_position(p, cx)` |
| `getSelections()` / `setSelections(s)` | `selections()` / `set_selections(&s, cx)`, `set_selections_from(source, &s, cx)` (as in Monaco, the setters don't scroll: `set_position(p, cx)` then `reveal_position_in_center(p, cx)`) |
| `revealLine`, `revealLineInCenter`, `revealLines…`, `revealRange…`, `revealPosition…` | `reveal_line`, `reveal_line_in_center`, `reveal_lines…`, `reveal_range…`, `reveal_position…` |
| `revealRange(range, ScrollType.Immediate)` | `reveal_range_with_scroll_type(range, RevealType::Simple, ScrollType::Immediate, cx)`; `reveal_lines_with_scroll_type` for lines |
| `revealAllCursors(revealHorizontal, minimalReveal)` | `reveal_all_cursors(reveal_horizontal, minimal_reveal, cx)` |
| `getScrollTop()` / `setScrollTop(y, scrollType)` | `scroll_top()` / `set_scroll_top(y, ScrollType::…, cx)`, likewise `scroll_left`, and `scroll_position()` / `set_scroll_position(p, cx)` |
| `hasPendingScrollAnimation()` | `has_pending_scroll_animation()` |
| `getLayoutInfo()` | `layout_info()` (a `LayoutInfo`, after the first frame) |
| `getTopForLineNumber(n, includeViewZones)`, `getBottomForLineNumber(n)`, `getTopForPosition(n, c)` | `top_for_line_number(row, include_view_zones)`, `bottom_for_line_number(row, include_view_zones)`, `top_for_position(p)` |
| `getLineHeightForPosition(p)` | `line_height_for_position(p)` (0 in a folded region) |
| `getOffsetForColumn(n, c)`, `getWidthOfLine(n)`, `getVisibleColumnFromPosition(p)` | `offset_for_column(row, column)`, `width_of_line(row)`, `visible_column_from_position(p, cx)` (0-based) |
| `getStatusbarColumn(p)` | `statusbar_column(p, cx)` (1-based, as in Monaco) |
| `inComposition` | `in_composition()` |
| `getContentWidth()`, `getContentHeight()`, `getScrollWidth()`, `getScrollHeight()` | `content_width()`, `content_height()`, `scroll_width()`, `scroll_height()` |
| `getVisibleRanges()`, `getVisibleRangesPlusViewportAboveBelow()` | `visible_ranges(cx)`, `visible_ranges_plus_viewport_above_below(cx)` |
| `getScrolledVisiblePosition(p)` | `scrolled_visible_position_in_editor(p)` (`top`, `left`, `height` from the editor's top-left), or `scrolled_visible_position(p)` in window coordinates |
| `getTargetAtClientPoint(x, y)` | `mouse_target(point, window, cx)` |
| `focus()` / `hasTextFocus()` / `hasWidgetFocus()` | `window.focus(&editor.focus_handle(cx), cx)` / `has_text_focus(window)` / `has_widget_focus(window, cx)` |
| `saveViewState()` / `restoreViewState(s)` | `save_view_state(cx)` / `restore_view_state(&s, cx)` (with the folded regions) |
| `executeEdits(source, edits, endCursorState)` | `execute_edits_from(source, edits, end_selections, cx)` (or `execute_edits(edits, end_selections, cx)` for `"api"`), or `execute_edit_operations_from(source, edits, end_cursor_state, cx)` with `SingleEditOperation`s (`force_move_markers`) and an `EndCursorState` (selections or a cursor state computer) |
| `pushUndoStop()` / `popUndoStop()` | `push_undo_stop(cx)` / `pop_undo_stop(cx)` |
| `trigger(source, handlerId, payload)` | `trigger_from(source, id, payload, window, cx)`, or `trigger(id, window, cx)` / `trigger_with_payload(id, json, window, cx)` for `"keyboard"`, taking any Monaco action, command or handler id (`cursorDown`, `type`, `cursorMove`, `editor.fold`, aliases, …) with Monaco's payload, or a GPUI action name; `DiffEditor::trigger` for `diffEditor.*` ids |
| `getAction(id)`, `getActions()` | `get_action(id, cx)` (an `EditorAction` with `id()`, `label()`, `alias()`, `is_supported(cx)` and `run(args, window, cx)`), `actions()` |
| `getAction(id).run(args)` | `run_action(id, window, cx)`, `run_action_with_args(id, args, window, cx)` |
| `getId()`, `getEditorType()`, `EditorType` | `id()` (`vs.editor.ICodeEditor:1`), `editor_type()`, `EditorType` (`DiffEditor` has them too) |
| `addAction(descriptor)` | `add_action(EditorActionDescriptor::new(id, label, run).keybinding(…).keybinding_context(when), cx)`, returning a `Disposable`; its keybindings apply in this editor only; `EditorActionDescriptor::new_with_args` for `run(editor, ...args)` |
| `addCommand(keybinding, handler, context)` | `add_command(keystrokes, handler, when, cx)`, returning the command's id and a `Disposable`; its keybinding applies in this editor only |
| `createContextKey(key, defaultValue)` | `create_context_key(key, default, cx)`; `ContextKey::set`, `reset`, `get` |
| `executeCommand(source, command)` / `executeCommands(source, commands)` | `execute_command(source, command, cx)` / `execute_commands(source, commands, cx)` with an `EditorCommand` |
| `when` clauses over context keys | GPUI predicates over `context_keys(cx)` (`CodeEditor && editorTextFocus && editorLangId == rust`) |
| `getSupportedActions()` | `supported_actions(cx)` |
| `createDecorationsCollection(d)` | `create_decorations_collection(d, cx)` |
| `deltaDecorations(old, new)`, `removeDecorations(ids)` | `delta_decorations(&old, new, cx)`, `remove_decorations(&ids, cx)` (editor-owned) |
| `getLineDecorations(n)`, `getDecorationsInRange(range)` | `line_decorations(row, cx)`, `decorations_in_range(range, cx)` |
| `setHiddenAreas(ranges, source)` | `set_hidden_areas(rows, cx)`, `set_hidden_areas_from(source, rows, cx)` (0-based rows, end exclusive) |
| `changeViewZones(a => …)` | `change_view_zones(\|zones\| …, cx)` with `add_zone`, `remove_zone`, `layout_zone`, `set_zone_height`; or one at a time: `add_view_zone(zone, cx)`, `layout_view_zone`, `set_view_zone_height`, `remove_widget` |
| `addContentWidget(w)` / `layoutContentWidget(w)` | `add_content_widget(w, cx)` / `set_content_widget_position(…)` |
| `addOverlayWidget(w)` | `add_overlay_widget(w, cx)` |
| `addGlyphMarginWidget(w)` / `layoutGlyphMarginWidget(w)` | `add_glyph_margin_widget(GlyphMarginWidget { range, view, lane, z_index }, cx)` (`GlyphMarginWidget::new(row, view)` for a line) / `layout_glyph_margin_widget(id, GlyphMarginWidgetPosition { range, lane, z_index }, cx)` |
| `getContribution(id)` | built-in contributions are `CodeEditor` methods (the find controller's are `start_find`, …); a host's own per-editor state goes in `add_contribution(value)` and comes back by type from `get_contribution::<T>()` |
| `getFontSizeAtPosition(p)` | `get_font_size_at_position(p)` (`Pixels`; the editor's size, with the zoom) |
| `writeScreenReaderContent(reason)`, `setAriaOptions({ activeDescendant, role })` | `write_screen_reader_content(reason, cx)`, `set_aria_options(AriaOptions { active_descendant, role }, cx)` (an AccessKit node id and role) |
| `getFoldingModel()…` | `folding_ranges(cx)`, `collapsed_folds(cx)` |

### Editor events

| Monaco | `CodeEditorEvent` |
|---|---|
| `onDidChangeCursorSelection` / `onDidChangeCursorPosition` | `SelectionsChanged(SelectionChange { reason, source, selection, secondary_selections, old_selections, … })` / `CursorPositionChanged(CursorPositionChange)`, only when the selections changed |
| `onDidScrollChange` | `ScrollChanged(ScrollEvent)`, only when something changed |
| `onDidFocusEditorText` / `onDidBlurEditorText` | `Focused` / `Blurred` |
| `onMouseDown`, `onMouseUp`, `onMouseMove`, `onMouseLeave`, `onContextMenu` | `MouseDown`, `MouseUp`, `MouseMove`, `MouseLeave`, `ContextMenu` (with `EditorMouseEvent`; `MouseLeave` with a `PartialEditorMouseEvent`) |
| `onDidAttemptReadOnlyEdit` | `ReadOnlyEditAttempt` |
| `onWillChangeModel` / `onDidChangeModel` | `WillChangeModel` / `ModelChanged` (`ModelChange { old_uri, new_uri }`) |
| `onDidChangeModelLanguage` | `ModelLanguageChanged { old_language, new_language, source }` |
| `onDidChangeLineHeight`, `onDidChangeFont` | `LineHeightChanged(changes)`, `FontChanged(changes)` (the decorations this editor shows) |
| `onDropIntoEditor`, `onWillTriggerEditorOperationEvent` | `DropIntoEditor(DropIntoEditorEvent { position, data })`, `WillTriggerEditorOperation(TriggerEditorOperationEvent { source, handler_id, payload })` |
| `onDidChangeConfiguration` | `ConfigurationChanged(change)`; `change.has_changed(\|o\| &o.font_size)` |
| `onDidLayoutChange` / `onDidContentSizeChange` | `LayoutChanged(LayoutInfo)` / `ContentSizeChanged` |
| `onDidFocusEditorWidget` / `onDidBlurEditorWidget` | `WidgetFocused` / `WidgetBlurred` |
| `onWillType` / `onDidType` | `WillType` / `DidType` |
| `onWillPaste` / `onDidPaste`, `onWillCopy`, `onWillCut` | `WillPaste` / `DidPaste`, `WillCopy`, `WillCut` (notifications: the clipboard data can't be changed) |
| `onKeyDown` / `onKeyUp`, `onMouseWheel` | `KeyDown` / `KeyUp` (a gpui `Keystroke`; can't be prevented), `MouseWheel` |
| `onMouseDrag`, `onMouseDrop`, `onMouseDropCanceled` | `MouseDrag`, `MouseDrop`, `MouseDropCanceled` |
| `onDidCompositionStart` / `onDidCompositionEnd` | `CompositionStarted` / `CompositionEnded` |
| `onDidChangeHiddenAreas`, `onDidChangeViewZones` | `HiddenAreasChanged` (after the next frame), `ViewZonesChanged` |
| `onBeginUpdate` / `onEndUpdate` | `BeginUpdate` / `EndUpdate` |
| `onDidChangeModelContent`, `onDidChangeModelDecorations`, `onDidChangeModelOptions`, `onDidChangeModelTokens`, `onDidChangeModelLanguageConfiguration` | `ModelContentChanged(ModelContentChange)`, `ModelDecorationsChanged`, `ModelOptionsChanged`, `ModelTokensChanged { ranges }`, `ModelLanguageConfigurationChanged` |
| opener service | the editor opens web and mail links in the browser, runs `command:` links and follows links into its own model; `OpenLocation(location)` and `OpenLink(url)` report what no `register_editor_opener`/`register_link_opener` opener, nor the editor, opened |
| `commandService.executeCommand` from providers | `ExecuteCommand(command)`, or `register_command` |

### Global events

Monaco's global `monaco.editor.on…` events are free functions returning a gpui `Subscription`; dropping it unsubscribes (`IDisposable.dispose()`), and `detach()` keeps it.

| Monaco | gpui-monaco |
|---|---|
| `onDidCreateEditor`, `onDidCreateDiffEditor` | `on_did_create_editor(cx, …)`, `on_did_create_diff_editor(cx, …)` |
| `onDidCreateModel`, `onWillDisposeModel` | `on_did_create_model(cx, …)`, `on_will_dispose_model(cx, …)` (the model's id and state, as it's released) |
| `model.onWillDispose`, `model.onDidChangeTokens` | `TextModel::on_will_dispose(&model, cx, …)`, `TextModelEvent::SyntaxUpdated { ranges }` |
| `CommandsRegistry.registerCommand(id, handler)` | `register_command(cx, id, handler)`, returning a `Disposable`; the newest handler for an id runs |
| `onDidChangeModelLanguage` | `on_did_change_model_language(cx, …)` (the model and its old language id) |
| `onDidChangeMarkers` | `on_did_change_markers(cx, …)` (the changed uris) |

## Keybindings

Keystrokes are GPUI strings, not `KeyMod | KeyCode` numbers:

| Monaco | GPUI keystroke |
|---|---|
| `KeyMod.CtrlCmd` | `secondary-` (Cmd on macOS, Ctrl elsewhere). Use it rather than `cmd-` or `ctrl-` so a rule works on every platform, as Monaco's does |
| `KeyMod.WinCtrl` | `ctrl-` (Ctrl on every platform) |
| `KeyMod.Shift`, `KeyMod.Alt` | `shift-`, `alt-` |
| `KeyCode.KeyK`, `KeyCode.Digit1`, `KeyCode.F12` | `k`, `1`, `f12` |
| `KeyCode.Enter`, `Escape`, `Tab`, `Backspace`, `Delete`, `UpArrow`, `PageDown` | `enter`, `escape`, `tab`, `backspace`, `delete`, `up`, `pagedown` |
| `KeyMod.chord(KeyMod.CtrlCmd \| KeyCode.KeyK, KeyMod.CtrlCmd \| KeyCode.KeyM)` | `"secondary-k secondary-m"` (space-separated keystrokes) |
| `when` clause | a GPUI key context predicate over `context_keys(cx)` |

So `editor.addCommand(KeyMod.CtrlCmd | KeyCode.KeyS, save)` becomes `editor.add_command("secondary-s", save, None, cx)`. [Keybindings](keymap.md) covers rebinding and removing the defaults.

## Language features

Monaco's `monaco.languages.register…Provider(selector, provider)` functions become `providers::register_…_provider(cx, LanguageSelector, provider)`, where each provider is a Rust trait. `providers::unregister_provider(cx, registration)` replaces `IDisposable.dispose()`. Matching providers are ranked as in Monaco: by selector score, then newest first.

| Monaco | gpui-monaco trait |
|---|---|
| `registerCompletionItemProvider` | `CompletionProvider` |
| `registerHoverProvider` | `HoverProvider` |
| `registerSignatureHelpProvider` | `SignatureHelpProvider` |
| `registerDefinitionProvider`, `…Declaration…`, `…TypeDefinition…`, `…Implementation…` | `DefinitionProvider`, `DeclarationProvider`, `TypeDefinitionProvider`, `ImplementationProvider` |
| `registerReferenceProvider` | `ReferenceProvider` |
| `registerDocumentHighlightProvider` | `DocumentHighlightProvider` |
| `registerRenameProvider` | `RenameProvider` |
| `registerNewSymbolNameProvider` | `NewSymbolNamesProvider` (`register_new_symbol_name_provider`), the rename box's suggestions |
| `registerCodeActionProvider` | `CodeActionProvider` |
| `registerCodeLensProvider` | `CodeLensProvider` |
| `registerInlayHintsProvider` | `InlayHintsProvider` |
| `registerInlineCompletionsProvider` | `InlineCompletionsProvider` |
| `registerDocumentFormattingEditProvider`, `…RangeFormatting…`, `…OnTypeFormatting…` | `DocumentFormattingProvider`, `DocumentRangeFormattingProvider`, `OnTypeFormattingProvider` |
| `registerDocumentSymbolProvider` | `DocumentSymbolProvider` |
| `registerFoldingRangeProvider` | `FoldingRangeProvider` (given Monaco's empty `FoldingContext`) |
| `registerSelectionRangeProvider` | `SelectionRangeProvider` |
| `registerLinkedEditingRangeProvider` | `LinkedEditingRangeProvider` |
| `registerLinkProvider` | `LinkProvider` |
| `registerColorProvider` | `DocumentColorProvider` |
| `registerDocumentSemanticTokensProvider`, `…RangeSemanticTokens…` | `DocumentSemanticTokensProvider`, `DocumentRangeSemanticTokensProvider` |
| `registerDocumentDropEditProvider`, `registerDocumentPasteEditProvider` | `DocumentDropEditProvider`, `DocumentPasteEditProvider` |
| `setLanguageConfiguration` | the language's configuration in the `LanguageRegistry` (`gpui-monaco-languages` reads Monaco's JSON), or at runtime `set_language_configuration(cx, id, config)`, which layers it over the language's and returns a `Disposable` |
| `register({ id, extensions, filenames, filenamePatterns, firstLine, mimetypes })` | `Language::new(id, name)` with `with_extensions`, `with_file_names`, `with_filename_patterns`, `with_first_line` and `with_mimetypes`, registered with `register_language(cx, language)`, which updates models already using or guessing it (or `LanguageRegistry::register` before any model resolves its language); registering an id again merges into it, as in Monaco |
| `onLanguage(id, callback)`, `onLanguageEncountered(id, callback)` | `on_language(cx, id, callback)`, `on_language_encountered(cx, id, callback)`, returning a `Disposable` |
| `LanguageFilter { language, scheme, pattern, exclusive }`, `'*'` | `LanguageSelector::Filters(vec![LanguageFilter { … }])`, `LanguageSelector::All` or `Languages(vec!["*".into()])` |
| `languages.score(selector, uri, languageId)` | `selector.score(&target)` or `providers::score` |
| `FormattingConflicts.setFormatterSelector` (not public in Monaco's API) | `set_formatter_selector(cx, …)` |
| `setMonarchTokensProvider` | `set_monarch_tokens_provider(cx, id, &grammar)` with the grammar as JSON, or `Language::with_monarch_grammar`; a tree-sitter grammar with highlight queries also works (see [Languages](languages.md#monarch-grammars)) |
| `setTokensProvider`, `registerTokensProviderFactory` | `set_tokens_provider(cx, id, provider)` with a `syntax::TokensProvider`, `register_tokens_provider_factory(cx, id, factory)` |
| `Location`, `LocationLink` | `Location`, with `origin_selection_range` and `target_selection_range` for a link |
| `WorkspaceEdit` (text, file and snippet edits, `versionId`, `metadata`) | `WorkspaceEdit` with `edits`, `file_edits` (applied by `set_workspace_file_edit_handler`), `snippet_edits`, `versions` and `metadata` |
| `editor.remeasureFonts` | `remeasure_fonts(cx)`, a no-op: GPUI measures text every frame |
| `executeCommand('_executeLinkProvider', uri)`, `_executeSignatureHelpProvider`, `_executeSelectionRangeProvider`, `_executeCompletionItemProvider`, `_provideDocument(Range)SemanticTokens(Legend)` | `api_commands::execute_link_provider(&model, resolve_count, cx)` (the built-in URL links included, merged and sorted as Monaco's `getLinks`) and the others, returning a `Task` with the providers' answers |
| `editor.setColorMap`, `languages.getEncodedLanguageId`, `EncodedTokensProvider` | none: there are no encoded tokens; themes resolve token names |

The `gpui-monaco-lsp` bridge registers these providers from a language server (see [Language servers](lsp.md)).

## Options

`IEditorOptions` becomes the typed `EditorOptions` struct with snake_case names, such as `wordWrap` → `word_wrap` and `minimap.enabled` → `minimap.enabled`. [Options](options.md) lists them with their Monaco names. The model-level options (`tabSize`, `insertSpaces`, …) are on `ModelOptions`.
