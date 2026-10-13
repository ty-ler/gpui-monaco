//! `PLAYGROUND_SCREENSHOT=path.png` support (requires `--features screenshot`).
//!
//! `PLAYGROUND_KEYS` optionally scripts input before the capture: whitespace
//! separated keystrokes in GPUI syntax (`cmd-down shift-up`), where a token
//! `text:abc` types `abc` character by character (`_` types a space),
//! `wheel:x,y,dy` moves the mouse to `x, y` (window pixels) and turns the
//! wheel by `dy` pixels there, `click:x,y` clicks there with the left
//! button, `mouse:x,y` moves the mouse to `x, y` (`mouse:x,y,secondary`
//! with Cmd/Ctrl held, as for a Cmd/Ctrl+hover link), `down:x,y` presses
//! the left button there and `drag:x,y` moves the mouse with it held (a
//! drag in progress), and `wait` pauses 150ms and draws a frame.

use gpui::{AnyWindowHandle, App};

#[cfg(feature = "screenshot")]
pub fn capture_if_requested(window: AnyWindowHandle, cx: &mut App) {
    use gpui::AppContext as _;
    use std::time::Duration;

    let Ok(path) = std::env::var("PLAYGROUND_SCREENSHOT") else {
        return;
    };
    let keys = std::env::var("PLAYGROUND_KEYS").unwrap_or_default();
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(300))
            .await;
        for token in keys.split_whitespace() {
            if token == "wait" {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                // Draw a frame as a visible window would, so work started
                // while painting (e.g. requests for providers that just
                // registered) gets going.
                cx.update_window(window, |_, window, cx| {
                    window.refresh();
                    window.draw(cx).clear(cx);
                })
                .ok();
                continue;
            }
            if let Some(args) = token.strip_prefix("mouse:") {
                let mut parts = args.split(',');
                let mut number = || -> f32 {
                    parts
                        .next()
                        .and_then(|n| n.parse().ok())
                        .expect("mouse:x,y[,secondary]")
                };
                let position = gpui::point(gpui::px(number()), gpui::px(number()));
                let modifiers = match parts.next() {
                    Some("secondary") => gpui::Modifiers::secondary_key(),
                    Some(other) => panic!("unknown mouse modifier {other}"),
                    None => gpui::Modifiers::default(),
                };
                cx.update_window(window, |_, window, cx| {
                    window.dispatch_event(
                        gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                            position,
                            modifiers,
                            ..Default::default()
                        }),
                        cx,
                    );
                })
                .ok();
                continue;
            }
            // A left press held through the following `drag:` moves (to
            // capture a drag in progress, such as the drop line).
            if let Some((kind, args)) = token
                .strip_prefix("down:")
                .map(|args| ("down", args))
                .or_else(|| token.strip_prefix("drag:").map(|args| ("drag", args)))
            {
                let [x, y]: [f32; 2] = args
                    .split(',')
                    .map(|n| n.parse().expect("invalid mouse number"))
                    .collect::<Vec<_>>()
                    .try_into()
                    .expect("down:x,y or drag:x,y");
                let position = gpui::point(gpui::px(x), gpui::px(y));
                cx.update_window(window, |_, window, cx| {
                    let event = if kind == "down" {
                        window.dispatch_event(
                            gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                                position,
                                ..Default::default()
                            }),
                            cx,
                        );
                        window.refresh();
                        window.draw(cx).clear(cx);
                        gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                            button: gpui::MouseButton::Left,
                            position,
                            click_count: 1,
                            ..Default::default()
                        })
                    } else {
                        gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                            position,
                            pressed_button: Some(gpui::MouseButton::Left),
                            ..Default::default()
                        })
                    };
                    window.dispatch_event(event, cx);
                })
                .ok();
                continue;
            }
            if let Some(args) = token.strip_prefix("click:") {
                let [x, y]: [f32; 2] = args
                    .split(',')
                    .map(|n| n.parse().expect("invalid click number"))
                    .collect::<Vec<_>>()
                    .try_into()
                    .expect("click:x,y");
                let position = gpui::point(gpui::px(x), gpui::px(y));
                cx.update_window(window, |_, window, cx| {
                    window.dispatch_event(
                        gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                            position,
                            ..Default::default()
                        }),
                        cx,
                    );
                    window.refresh();
                    window.draw(cx).clear(cx);
                    window.dispatch_event(
                        gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                            button: gpui::MouseButton::Left,
                            position,
                            click_count: 1,
                            ..Default::default()
                        }),
                        cx,
                    );
                    window.dispatch_event(
                        gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                            button: gpui::MouseButton::Left,
                            position,
                            click_count: 1,
                            ..Default::default()
                        }),
                        cx,
                    );
                })
                .ok();
                continue;
            }
            if let Some(args) = token.strip_prefix("wheel:") {
                let [x, y, dy]: [f32; 3] = args
                    .split(',')
                    .map(|n| n.parse().expect("invalid wheel number"))
                    .collect::<Vec<_>>()
                    .try_into()
                    .expect("wheel:x,y,dy");
                let position = gpui::point(gpui::px(x), gpui::px(y));
                cx.update_window(window, |_, window, cx| {
                    window.dispatch_event(
                        gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                            position,
                            ..Default::default()
                        }),
                        cx,
                    );
                    // Hit testing goes by the last frame: draw one with the
                    // mouse there first.
                    window.refresh();
                    window.draw(cx).clear(cx);
                    window.dispatch_event(
                        gpui::PlatformInput::ScrollWheel(gpui::ScrollWheelEvent {
                            position,
                            delta: gpui::ScrollDelta::Pixels(gpui::point(
                                gpui::px(0.),
                                gpui::px(dy),
                            )),
                            ..Default::default()
                        }),
                        cx,
                    );
                })
                .ok();
                continue;
            }
            let keystrokes: Vec<gpui::Keystroke> = match token.strip_prefix("text:") {
                Some(text) => text
                    .chars()
                    .map(|ch| {
                        let ch = if ch == '_' { ' ' } else { ch };
                        gpui::Keystroke {
                            modifiers: Default::default(),
                            key: ch.to_string(),
                            key_char: Some(ch.to_string()),
                        }
                    })
                    .collect(),
                None => vec![gpui::Keystroke::parse(token).expect("invalid keystroke")],
            };
            for keystroke in keystrokes {
                cx.update_window(window, |_, window, cx| {
                    window.dispatch_keystroke(keystroke, cx);
                })
                .ok();
            }
        }
        cx.background_executor()
            .timer(Duration::from_millis(450))
            .await;
        // A background window may not have redrawn since the last input, so
        // draw a fresh frame before capturing it.
        let result = cx.update_window(window, |_, window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
            window.render_to_image()
        });
        match result {
            Ok(Ok(image)) => {
                image.save(&path).expect("failed to save screenshot");
                eprintln!("saved screenshot to {path}");
            }
            Ok(Err(error)) => eprintln!("render_to_image failed: {error:#}"),
            Err(error) => eprintln!("window gone: {error:#}"),
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

#[cfg(not(feature = "screenshot"))]
pub fn capture_if_requested(_window: AnyWindowHandle, _cx: &mut App) {
    if std::env::var_os("PLAYGROUND_SCREENSHOT").is_some() {
        eprintln!("PLAYGROUND_SCREENSHOT requires `--features screenshot`");
    }
}
