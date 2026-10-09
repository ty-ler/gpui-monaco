//! `PLAYGROUND_SCREENSHOT=path.png` support (requires `--features screenshot`).
//!
//! `PLAYGROUND_KEYS` optionally scripts input before the capture: whitespace
//! separated keystrokes in GPUI syntax (`cmd-down shift-up`), where a token
//! `text:abc` types `abc` character by character (`_` types a space) and
//! `wait` pauses 150ms.

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
