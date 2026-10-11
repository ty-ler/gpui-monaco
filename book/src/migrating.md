# Migrating from Monaco

This page maps Monaco's API to gpui-monaco for people porting a Monaco integration. The shapes follow Rust and GPUI:

- **Models and editors are entities.** Create them with `cx.new`, and read or change them through `read`/`update`.
- **Positions are 0-based.** They're `PointUtf16 { row, column }` with UTF-16 columns, where Monaco uses 1-based `{ lineNumber, column }`. `PointUtf16::from_monaco` and `to_monaco` convert.
- **Events are GPUI events.** Subscribe with `cx.subscribe(&editor, …)` instead of `onDid…` listeners.
- **Async results are `Task`s.** Dropping a task cancels it, so there are no cancellation tokens.

[Differences from Monaco](differences.md) lists behavior that differs.

## Creating editors and models

| Monaco | gpui-monaco |
|---|---|
| `monaco.editor.create(el, { value, language })` | `cx.new(\|cx\| TextModel::new(value).with_language(language))`, then `CodeEditor::new(model, window, cx)` |
| `monaco.editor.createModel(value, language, uri)` | `TextModel::new(value).with_language(language).with_uri(uri)`, then `register_model` so providers can find it |
| `monaco.editor.getModel(uri)` | `model_for_uri(cx, uri)` |
| `monaco.editor.getModels()`, `getEditors()`, `getDiffEditors()` | `models(cx)`, `editors(cx)`, `diff_editors(cx)` |
| `monaco.editor.setModelLanguage(model, id)` | `model.set_language(id, cx)` |
| `monaco.editor.setModelMarkers(model, owner, markers)` | `model.set_markers(owner, markers, cx)` |
| `monaco.editor.getModelMarkers({ owner, resource, take })` | `model_markers(cx, &MarkerFilter { owner, resource, take })` |
| `monaco.editor.removeAllMarkers(owner)` | `remove_all_markers(cx, owner)` |
| `monaco.editor.defineTheme(name, data)` | `define_theme(cx, Theme::define(name, &base, inherit, rules, &colors))`, or a theme from `Theme::from_vscode_json` |
| `monaco.editor.setTheme(name)` | `set_theme(cx, name)` (every editor), or `editor.set_theme(Arc::new(theme), cx)` (one) |
| `monaco.editor.colorize(text, lang, opts)` | `colorize::colorize(text, lang, &theme, &registry, opts)` or `colorize_element` |
| `monaco.editor.tokenize(text, lang)` | `colorize::tokenize(text, lang, &registry)` |
| `monaco.editor.colorizeModelLine(model, line, tabSize)` | `colorize::colorize_model_line(model.read(cx), row, tab_size, &theme, cx)` |
| `monaco.editor.addCommand({ id, run })` | `add_command(cx, id, handler)`, returning a `Disposable` |
| `monaco.editor.addEditorAction(descriptor)` | `add_editor_action(cx, EditorActionDescriptor::new(…))` |
| `monaco.editor.addKeybindingRule(s)(rules)` | `add_keybinding_rule(s)(cx, KeybindingRule::new(keystrokes, command))` |
| `monaco.languages.register(...)` | `set_language_registry(cx, registry)`, with grammars from `gpui-monaco-languages` |
| `editor.dispose()` / `model.dispose()` | drop the entity |
| `editor.layout()` | not needed: the editor lays itself out every frame |

## Text model

| Monaco `ITextModel` | `TextModel` |
|---|---|
| `getValue()` / `setValue(v)` | `text()` / `set_value(v, cx)` |
| `getLineCount()`, `getLineContent(n)`, `getLineLength(n)` | `line_count()`, `line_content(row)`, `line_len_utf16(row)` |
| `getOffsetAt(pos)` / `getPositionAt(offset)` | `offset_at(point)` / `position_at(offset)` (byte offsets) |
| `getValueInRange(range)` | `value_in_range(range)` |
| `validatePosition`, `validateRange` | `validate_position`, `validate_range` |
| `getWordAtPosition`, `getWordUntilPosition` | `word_at_position`, `word_until_position` |
| `findMatches(...)` | `find_matches(query, is_regex, match_case, whole_word, …)`, or `find_matches_in(query, scope, …, capture_matches, limit)` |
| `findNextMatch(...)` / `findPreviousMatch(...)` | `find_next_match(...)` / `find_previous_match(...)`, returning a `FindMatch` |
| `getLinesContent()`, `getValueLength()`, `getFullModelRange()` | `lines_content()`, `value_length()`, `full_model_range()` |
| `modifyPosition(pos, offset)` | `modify_position(point, offset)` |
| `applyEdits(edits, true)` | `apply_edits(edits, cx)`, returning the undo edits (also recorded for undo) |
| `pushEditOperations(before, edits, computer)` | `push_edit_operations(before, edits, computer, cx)`, or `edit(edits, cx)` grouped with `start_transaction`/`end_transaction` |
| `pushStackElement()` / `popStackElement()` | `push_stack_element()` / `pop_stack_element()` |
| `getAlternativeVersionId()` | `alternative_version_id()` |
| `bracketPairs.getBracketPairsInRange`, `matchBracket`, `findEnclosingBrackets` | `bracket_pairs_in_range`, `match_bracket`, `find_enclosing_brackets` |
| `guides.getLinesIndentGuides`, `getActiveIndentGuide` | `lines_indent_guides`, `active_indent_guide` |
| `undo()` / `redo()`, `canUndo()` / `canRedo()` | `undo(source, cx)` / `redo(source, cx)`, `can_undo()` / `can_redo()` |
| `getEOL()` / `setEOL(...)` / `pushEOL(...)` | `line_ending()` / `set_line_ending(...)` / `push_line_ending(...)` (an undo step) |
| `isAttachedToEditor()`, `onDidChangeAttached` | `is_attached_to_editor()`, `TextModelEvent::AttachedChanged` |
| `normalizeIndentation(s)` | `normalize_indentation(s)` |
| `getLineFirstNonWhitespaceColumn(n)`, `getLineLastNonWhitespaceColumn(n)` | `line_first_non_whitespace_column(row)`, `line_last_non_whitespace_column(row)` (0-based, `None` for blank lines) |
| `tokenization.getLineTokens(n)` | `line_tokens(row, cx)` (`colorize::Token`s) |
| `getOptions()` / `updateOptions(...)` | `options()` / `update_options(...)` (tab size, insert spaces, …) |
| `deltaDecorations(old, new)` | `delta_decorations(owner, &old, new, cx)` (byte ranges) |
| `getDecorationsInRange(range)` | `decorations_in_range(range, editor)` |
| `onDidChangeContent` | `TextModelEvent::Edited { changes, version, … }` |
| `onDidChangeLanguage`, `onDidChangeOptions`, `onDidChangeDecorations` | `LanguageChanged { old_language, new_language }`, `OptionsChanged`, `DecorationsChanged` |

## Editor

| Monaco `ICodeEditor` | `CodeEditor` |
|---|---|
| `getModel()` / `setModel(m)` | `model()` / `set_model(m, cx)` |
| `getValue()` / `setValue(v)` | through the model: `model().read(cx).text()`, `set_value` |
| `getOptions()` / `updateOptions(o)` | `options()` / `update_options(full_options, cx)`: clone, change, set |
| `getPosition()` / `setPosition(p)` | `selections()[0].head` / `set_selections(&[Selection::collapsed(p)], cx)` |
| `getSelections()` / `setSelections(s)` | `selections()` / `set_selections(&s, cx)` |
| `revealLine`, `revealLineInCenter`, `revealLines…`, `revealRange…`, `revealPosition…` | `reveal_line`, `reveal_line_in_center`, `reveal_lines…`, `reveal_range…`, `reveal_position…` |
| `revealRange(range, ScrollType.Immediate)` | `reveal_range_with_scroll_type(range, RevealType::Simple, ScrollType::Immediate, cx)`; `reveal_lines_with_scroll_type` for lines |
| `revealAllCursors(revealHorizontal, minimalReveal)` | `reveal_all_cursors(reveal_horizontal, minimal_reveal, cx)` |
| `getScrollTop()` / `setScrollTop(y, scrollType)` | `scroll_top()` / `set_scroll_top(y, ScrollType::…, cx)`, likewise `scroll_left`, and `scroll_position()` / `set_scroll_position(p, cx)` |
| `hasPendingScrollAnimation()` | `has_pending_scroll_animation()` |
| `getLayoutInfo()` | `layout_info()` (a `LayoutInfo`, after the first frame) |
| `getTopForLineNumber(n, includeViewZones)`, `getBottomForLineNumber(n)`, `getTopForPosition(n, c)` | `top_for_line_number(row, include_view_zones)`, `bottom_for_line_number(row, include_view_zones)`, `top_for_position(p)` |
| `getOffsetForColumn(n, c)`, `getWidthOfLine(n)`, `getVisibleColumnFromPosition(p)` | `offset_for_column(row, column)`, `width_of_line(row)`, `visible_column_from_position(p, cx)` (0-based) |
| `getStatusbarColumn(p)` | `statusbar_column(p, cx)` (1-based, as in Monaco) |
| `inComposition` | `in_composition()` |
| `getContentWidth()`, `getContentHeight()`, `getScrollWidth()`, `getScrollHeight()` | `content_width()`, `content_height()`, `scroll_width()`, `scroll_height()` |
| `getVisibleRanges()` | `visible_ranges(cx)` |
| `getScrolledVisiblePosition(p)` | `scrolled_visible_position(p)` |
| `getTargetAtClientPoint(x, y)` | `mouse_target(point, window, cx)` |
| `focus()` / `hasTextFocus()` / `hasWidgetFocus()` | `window.focus(&editor.focus_handle(cx), cx)` / `has_text_focus(window)` / `has_widget_focus(window, cx)` |
| `saveViewState()` / `restoreViewState(s)` | `save_view_state(cx)` / `restore_view_state(&s, cx)` (with the folded regions) |
| `executeEdits(source, edits, endCursorState)` | `execute_edits(edits, end_selections, cx)` |
| `pushUndoStop()` | `push_undo_stop(cx)` |
| `trigger(source, handlerId, payload)` | `trigger(id, window, cx)`, taking a Monaco action or command id (`cursorDown`, `undo`, …) or a GPUI action name; `DiffEditor::trigger` for `diffEditor.*` ids |
| `getAction(id).run()` | `run_action(id, window, cx)` |
| `addAction(descriptor)` | `add_action(EditorActionDescriptor::new(id, label, run).keybinding(…), cx)` |
| `addCommand(keybinding, handler)` | define a GPUI action and bind it in `actions::KEY_CONTEXT`, or `add_command` plus `add_keybinding_rule` |
| `getSupportedActions()` | `supported_actions(cx)` |
| `createDecorationsCollection(d)` | `create_decorations_collection(d, cx)` |
| `deltaDecorations(old, new)` | the model's `delta_decorations(Some(editor_id), …)` (editor-owned) |
| `setHiddenAreas(ranges)` | `set_hidden_areas(rows, cx)` (0-based rows, end exclusive) |
| `changeViewZones(a => a.addZone(z))` | `add_view_zone(zone, cx)`, `set_view_zone_height`, `remove_widget` |
| `addContentWidget(w)` / `layoutContentWidget(w)` | `add_content_widget(w, cx)` / `set_content_widget_position(…)` |
| `addOverlayWidget(w)` | `add_overlay_widget(w, cx)` |
| `addGlyphMarginWidget(w)` | `add_glyph_margin_widget(GlyphMarginWidget { row, view }, cx)` |
| `getContribution('editor.contrib.findController')` | the find methods on `CodeEditor` (`start_find`, …) |
| `getFoldingModel()…` | `folding_ranges(cx)`, `collapsed_folds(cx)` |

### Editor events

| Monaco | `CodeEditorEvent` |
|---|---|
| `onDidChangeCursorSelection` / `onDidChangeCursorPosition` | `SelectionsChanged(SelectionChange { reason, source })` |
| `onDidScrollChange` | `ScrollChanged` |
| `onDidFocusEditorText` / `onDidBlurEditorText` | `Focused` / `Blurred` |
| `onMouseDown`, `onMouseUp`, `onMouseMove`, `onMouseLeave`, `onContextMenu` | `MouseDown`, `MouseUp`, `MouseMove`, `MouseLeave`, `ContextMenu` (with `EditorMouseEvent`) |
| `onDidAttemptReadOnlyEdit` | `ReadOnlyEditAttempt` |
| `onWillChangeModel` / `onDidChangeModel` | `WillChangeModel` / `ModelChanged` (`ModelChange { old_uri, new_uri }`) |
| `onDidChangeModelLanguage` | `ModelLanguageChanged { old_language, new_language }` |
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
| `onDidChangeModelContent` | subscribe to the model's `TextModelEvent::Edited` |
| opener service / `registerEditorOpener` | `OpenLocation(location)` and `OpenLink(url)`: the host opens them |
| `commandService.executeCommand` from providers | `ExecuteCommand(command)`, or `register_command` |

### Global events

Monaco's global `monaco.editor.on…` events are free functions returning a gpui `Subscription`; dropping it unsubscribes (`IDisposable.dispose()`), and `detach()` keeps it.

| Monaco | gpui-monaco |
|---|---|
| `onDidCreateEditor`, `onDidCreateDiffEditor` | `on_did_create_editor(cx, …)`, `on_did_create_diff_editor(cx, …)` |
| `onDidCreateModel`, `onWillDisposeModel` | `on_did_create_model(cx, …)`, `on_will_dispose_model(cx, …)` (the model's id and state, as it's released) |
| `onDidChangeModelLanguage` | `on_did_change_model_language(cx, …)` (the model and its old language id) |
| `onDidChangeMarkers` | `on_did_change_markers(cx, …)` (the changed uris) |

## Language features

Monaco's `monaco.languages.register…Provider(selector, provider)` functions become `providers::register_…_provider(cx, LanguageSelector, provider)`, where each provider is a Rust trait. `providers::unregister_provider(cx, registration)` replaces `IDisposable.dispose()`.

| Monaco | gpui-monaco trait |
|---|---|
| `registerCompletionItemProvider` | `CompletionProvider` |
| `registerHoverProvider` | `HoverProvider` |
| `registerSignatureHelpProvider` | `SignatureHelpProvider` |
| `registerDefinitionProvider`, `…Declaration…`, `…TypeDefinition…`, `…Implementation…` | `DefinitionProvider`, `DeclarationProvider`, `TypeDefinitionProvider`, `ImplementationProvider` |
| `registerReferenceProvider` | `ReferenceProvider` |
| `registerDocumentHighlightProvider` | `DocumentHighlightProvider` |
| `registerRenameProvider` | `RenameProvider` |
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
| `setLanguageConfiguration` | the language's configuration in the `LanguageRegistry` (`gpui-monaco-languages` reads Monaco's JSON), or at runtime `set_language_configuration(cx, id, config)` |
| `register({ id, extensions, filenames, filenamePatterns, firstLine, mimetypes })` | `Language::new(id, name)` with `with_extensions`, `with_file_names`, `with_filename_patterns`, `with_first_line` and `with_mimetypes` |
| `onLanguage(id, callback)` | `on_language(cx, id, callback)` |
| `LanguageFilter { language, scheme, pattern }` | `LanguageSelector::Filters(vec![LanguageFilter { … }])` |
| `FormattingConflicts.setFormatterSelector` (not public in Monaco's API) | `set_formatter_selector(cx, …)` |
| `setMonarchTokensProvider` | `set_monarch_tokens_provider(cx, id, &grammar)` with the grammar as JSON, or `Language::with_monarch_grammar`; a tree-sitter grammar with highlight queries also works (see [Languages](languages.md#monarch-grammars)) |
| `editor.remeasureFonts` | `remeasure_fonts(cx)`, a no-op: GPUI measures text every frame |
| `editor.setColorMap`, `languages.getEncodedLanguageId` | none: there are no encoded tokens; themes resolve token names |

The `gpui-monaco-lsp` bridge registers these providers from a language server (see [Language servers](lsp.md)).

## Options

`IEditorOptions` becomes the typed `EditorOptions` struct with snake_case names, such as `wordWrap` → `word_wrap` and `minimap.enabled` → `minimap.enabled`. [Options](options.md) lists them with their Monaco names. The model-level options (`tabSize`, `insertSpaces`, …) are on `ModelOptions`.
