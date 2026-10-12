//! Parameter hints (Monaco `contrib/parameterHints`): overload cycling, its
//! keys and commands, and the `parameterHints` options. Ports the
//! `parameterHintsModel.test.ts` cases our provider API can express.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

/// Lets a request through: Monaco asks the providers 120ms after typing,
/// cursor moves and edits (`ParameterHintsModel.DEFAULT_DELAY`).
fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(120));
    cx.run_until_parked();
}

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

/// Three overloads of `f` whenever the cursor is after a `(`, recording
/// each request's trigger.
#[derive(Clone, Default)]
struct Overloads {
    count: usize,
    triggers: Rc<RefCell<Vec<SignatureHelpTrigger>>>,
}

impl SignatureHelpProvider for Overloads {
    fn trigger_characters(&self) -> Vec<char> {
        vec!['(']
    }

    fn retrigger_characters(&self) -> Vec<char> {
        vec![',']
    }

    fn provide_signature_help(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        trigger: SignatureHelpTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<SignatureHelp>>> {
        self.triggers.borrow_mut().push(trigger);
        let line = context.snapshot.line(position.row);
        let before: String = line.chars().take(position.column as usize).collect();
        if !before.contains('(') || before.contains(')') {
            return Task::ready(Ok(None));
        }
        let signatures = (0..self.count)
            .map(|ix| SignatureInformation {
                label: format!("f{ix}(a)").into(),
                documentation: None,
                parameters: vec![ParameterInformation {
                    label: ParameterLabel::Text("a".into()),
                    documentation: None,
                }],
                active_parameter: None,
            })
            .collect();
        Task::ready(Ok(Some(SignatureHelp {
            signatures,
            active_signature: 0,
            active_parameter: 0,
        })))
    }
}

fn overloads(count: usize, cx: &mut TestAppContext) -> Rc<RefCell<Vec<SignatureHelpTrigger>>> {
    let provider = Overloads {
        count,
        ..Default::default()
    };
    let triggers = provider.triggers.clone();
    cx.update(|cx| {
        register_signature_help_provider(cx, LanguageSelector::All, provider);
    });
    triggers
}

fn active(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<usize> {
    editor.read_with(cx, |e, cx| {
        e.active_signature_for_tests(cx).map(|(_, ix, _)| ix)
    })
}

fn update_options(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    f: impl FnOnce(&mut gpui_monaco::EditorOptions),
) {
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        f(&mut options);
        e.update_options(options, cx);
    });
}

#[gpui::test]
fn up_and_down_cycle_through_overloads(cx: &mut TestAppContext) {
    overloads(3, cx);
    let (editor, cx) = setup("\nnext line", cx);
    cx.simulate_input("f(");
    settle(cx);
    assert_eq!(active(&editor, cx), Some(0));
    cx.simulate_keystrokes("down");
    assert_eq!(active(&editor, cx), Some(1));
    cx.simulate_keystrokes("alt-down");
    assert_eq!(active(&editor, cx), Some(2));
    // `cycle` (the default) wraps around both ways.
    cx.simulate_keystrokes("down");
    assert_eq!(active(&editor, cx), Some(0));
    cx.simulate_keystrokes("up");
    assert_eq!(active(&editor, cx), Some(2));
    cx.simulate_keystrokes("alt-up");
    assert_eq!(active(&editor, cx), Some(1));
    if cfg!(target_os = "macos") {
        cx.simulate_keystrokes("ctrl-n");
        assert_eq!(active(&editor, cx), Some(2));
        cx.simulate_keystrokes("ctrl-p");
        assert_eq!(active(&editor, cx), Some(1));
    }
    // The keys didn't move the cursor or the lines.
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0].head),
        PointUtf16::new(0, 2)
    );
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "f()\nnext line"
    );
    // Monaco's ids run them too.
    editor.update_in(cx, |e, window, cx| {
        e.trigger("showNextParameterHint", window, cx).unwrap()
    });
    settle(cx);
    assert_eq!(active(&editor, cx), Some(2));
    editor.update_in(cx, |e, window, cx| {
        e.trigger("showPrevParameterHint", window, cx).unwrap()
    });
    settle(cx);
    assert_eq!(active(&editor, cx), Some(1));
    editor.update_in(cx, |e, window, cx| {
        e.trigger("closeParameterHints", window, cx).unwrap()
    });
    settle(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_signature_help_visible()));
}

#[gpui::test]
fn without_cycle_the_ends_close_them(cx: &mut TestAppContext) {
    overloads(2, cx);
    let (editor, cx) = setup("", cx);
    update_options(&editor, cx, |o| o.parameter_hints.cycle = false);
    cx.simulate_input("f(");
    settle(cx);
    cx.simulate_keystrokes("down");
    assert_eq!(active(&editor, cx), Some(1));
    cx.simulate_keystrokes("down");
    assert_eq!(active(&editor, cx), None);

    cx.simulate_input("(");
    settle(cx);
    assert_eq!(active(&editor, cx), Some(0));
    cx.simulate_keystrokes("up");
    assert_eq!(active(&editor, cx), None);
}

/// Monaco's `showNextParameterHint` needs several signatures: with one,
/// Up/Down and Alt+Up/Down keep their usual meaning.
#[gpui::test]
fn one_signature_leaves_the_keys_alone(cx: &mut TestAppContext) {
    overloads(1, cx);
    let (editor, cx) = setup("first\n", cx);
    cx.simulate_keystrokes("down");
    cx.simulate_input("f(");
    settle(cx);
    assert_eq!(active(&editor, cx), Some(0));
    cx.simulate_keystrokes("alt-up");
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "f()\nfirst"
    );
}

#[gpui::test]
fn escape_and_shift_escape_close_them(cx: &mut TestAppContext) {
    overloads(2, cx);
    let (editor, cx) = setup("", cx);
    for key in ["escape", "shift-escape"] {
        cx.simulate_input("f(");
        settle(cx);
        assert!(editor.read_with(cx, |e, _| e.is_signature_help_visible()));
        cx.simulate_keystrokes(key);
        assert!(!editor.read_with(cx, |e, _| e.is_signature_help_visible()));
    }
}

/// `parameterHints.enabled: false`: typing doesn't open them, asking still
/// does (Monaco only turns off `triggerOnType`), and turning it off closes
/// them.
#[gpui::test]
fn disabled_hints_only_open_when_asked(cx: &mut TestAppContext) {
    overloads(2, cx);
    let (editor, cx) = setup("", cx);
    cx.simulate_input("f(");
    settle(cx);
    assert!(editor.read_with(cx, |e, _| e.is_signature_help_visible()));
    update_options(&editor, cx, |o| o.parameter_hints.enabled = false);
    assert!(!editor.read_with(cx, |e, _| e.is_signature_help_visible()));

    cx.simulate_input("g(");
    settle(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_signature_help_visible()));
    cx.simulate_keystrokes("secondary-shift-space");
    settle(cx);
    assert!(editor.read_with(cx, |e, _| e.is_signature_help_visible()));
}

/// Monaco: "Provider should get trigger character on type".
#[gpui::test]
fn provider_gets_the_trigger_character(cx: &mut TestAppContext) {
    let triggers = overloads(1, cx);
    let (_editor, cx) = setup("", cx);
    cx.simulate_input("(");
    settle(cx);
    assert_eq!(
        triggers.borrow().first(),
        Some(&SignatureHelpTrigger::TriggerCharacter('('))
    );
}

/// Monaco: "Provider should be retriggered by retrigger character": a
/// retrigger character alone does nothing, but does once hints show.
#[gpui::test]
fn retrigger_characters_need_visible_hints(cx: &mut TestAppContext) {
    let triggers = overloads(1, cx);
    let (_editor, cx) = setup("", cx);
    cx.simulate_input(",");
    settle(cx);
    assert!(triggers.borrow().is_empty());
    cx.simulate_input("f(");
    settle(cx);
    triggers.borrow_mut().clear();
    cx.simulate_input(",");
    settle(cx);
    assert!(
        triggers
            .borrow()
            .contains(&SignatureHelpTrigger::TriggerCharacter(','))
    );
}

/// Answers only with signature `label`, or nothing.
struct Fixed(Option<&'static str>);

impl SignatureHelpProvider for Fixed {
    fn trigger_characters(&self) -> Vec<char> {
        vec!['(']
    }

    fn provide_signature_help(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: SignatureHelpTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<SignatureHelp>>> {
        Task::ready(Ok(self.0.map(|label| SignatureHelp {
            signatures: vec![SignatureInformation {
                label: label.into(),
                documentation: None,
                parameters: Vec::new(),
                active_parameter: None,
            }],
            active_signature: 0,
            active_parameter: 0,
        })))
    }
}

/// Monaco: "should use first result from multiple providers".
#[gpui::test]
fn the_first_provider_with_a_result_wins(cx: &mut TestAppContext) {
    cx.update(|cx| {
        // Equal scores are tried newest first (Monaco `ordered`).
        register_signature_help_provider(cx, LanguageSelector::All, Fixed(Some("third")));
        register_signature_help_provider(cx, LanguageSelector::All, Fixed(Some("second")));
        register_signature_help_provider(cx, LanguageSelector::All, Fixed(None));
    });
    let (editor, cx) = setup("", cx);
    cx.simulate_input("(");
    settle(cx);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.active_signature_for_tests(cx)),
        Some(("second".to_string(), 0, 1))
    );
}

/// Monaco `ParameterHintsModel`'s `DEFAULT_DELAY`: typing asks the
/// providers 120ms after the last keystroke, with the triggers since then
/// merged (`mergeTriggerContexts`: a trigger character wins over content
/// changes).
#[gpui::test]
fn requests_wait_120ms_and_merge(cx: &mut TestAppContext) {
    let triggers = overloads(1, cx);
    let (editor, cx) = setup("", cx);
    cx.simulate_input("f(");
    cx.run_until_parked();
    assert!(triggers.borrow().is_empty(), "not yet");
    cx.executor().advance_clock(Duration::from_millis(119));
    cx.run_until_parked();
    assert!(triggers.borrow().is_empty(), "not yet");
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(
        *triggers.borrow(),
        [SignatureHelpTrigger::TriggerCharacter('(')]
    );
    assert_eq!(active(&editor, cx), Some(0));
    // Typing more within the delay asks once, for the last trigger
    // character.
    cx.simulate_input("x");
    cx.executor().advance_clock(Duration::from_millis(60));
    cx.simulate_input(",");
    cx.executor().advance_clock(Duration::from_millis(60));
    cx.run_until_parked();
    assert_eq!(triggers.borrow().len(), 1);
    settle(cx);
    assert_eq!(
        *triggers.borrow(),
        [
            SignatureHelpTrigger::TriggerCharacter('('),
            SignatureHelpTrigger::TriggerCharacter(',')
        ]
    );
    // Moving the cursor asks again as a content change.
    cx.simulate_keystrokes("left");
    settle(cx);
    assert_eq!(
        triggers.borrow().last(),
        Some(&SignatureHelpTrigger::ContentChange)
    );
    // An explicit request doesn't wait.
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("secondary-shift-space");
    cx.run_until_parked();
    assert_eq!(
        triggers.borrow().last(),
        Some(&SignatureHelpTrigger::Invoke)
    );
    assert_eq!(active(&editor, cx), Some(0));
}

/// A signature with the parameter `a` after an `a` in the name.
struct Named;

impl SignatureHelpProvider for Named {
    fn provide_signature_help(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: SignatureHelpTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<SignatureHelp>>> {
        Task::ready(Ok(Some(SignatureHelp {
            signatures: vec![SignatureInformation {
                label: "concat(cat: str, a: str)".into(),
                documentation: Some(Documentation::PlainText("Joins.".into())),
                parameters: vec![
                    ParameterInformation {
                        label: ParameterLabel::Text("cat".into()),
                        documentation: Some(Documentation::PlainText("The first.".into())),
                    },
                    ParameterInformation {
                        label: ParameterLabel::Text("a".into()),
                        documentation: None,
                    },
                ],
                active_parameter: None,
            }],
            active_signature: 0,
            active_parameter: 0,
        })))
    }
}

/// Monaco's providers have no trigger characters unless they declare
/// some (`signatureHelpTriggerCharacters` is optional): only an explicit
/// request opens the hints.
#[gpui::test]
fn providers_without_trigger_characters_open_only_when_asked(cx: &mut TestAppContext) {
    cx.update(|cx| register_signature_help_provider(cx, LanguageSelector::All, Named));
    let (editor, cx) = setup("", cx);
    cx.simulate_input("concat(");
    settle(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_signature_help_visible()));
    cx.simulate_keystrokes("secondary-shift-space");
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.is_signature_help_visible()));
}

/// Monaco `getParameterLabelOffsets`: a text label is found on word
/// boundaries, so `cat` is the parameter, not the end of `concat`; and
/// screen readers hear the parameter with its docs and the signature's
/// (`aria.alert("{0}, hint")`), once per parameter.
#[gpui::test]
fn the_active_parameter_matches_whole_words_and_is_announced(cx: &mut TestAppContext) {
    cx.update(|cx| register_signature_help_provider(cx, LanguageSelector::All, Named));
    let (editor, cx) = setup("", cx);
    cx.simulate_keystrokes("secondary-shift-space");
    cx.run_until_parked();
    let range = editor
        .read_with(cx, |e, cx| e.active_parameter_range_for_tests(cx))
        .unwrap();
    assert_eq!(range, 7..10);
    let heard = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| {
            e.last_announcement_for_tests().map(|m| m.to_string())
        })
    };
    assert_eq!(heard(cx).as_deref(), Some("cat, The first., Joins., hint"));
    // The same parameter isn't read again.
    editor.update(cx, |e, cx| e.announce("other", cx));
    cx.simulate_keystrokes("secondary-shift-space");
    cx.run_until_parked();
    assert_eq!(heard(cx).as_deref(), Some("other"));
}
