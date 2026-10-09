//! `PLAYGROUND_SCREENSHOT=path.png` support (requires `--features screenshot`).

use gpui::{AnyWindowHandle, App};

#[cfg(feature = "screenshot")]
pub fn capture_if_requested(window: AnyWindowHandle, cx: &mut App) {
    use gpui::AppContext as _;
    use std::time::Duration;

    let Ok(path) = std::env::var("PLAYGROUND_SCREENSHOT") else {
        return;
    };
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(750))
            .await;
        let result = cx.update_window(window, |_, window, _| window.render_to_image());
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
