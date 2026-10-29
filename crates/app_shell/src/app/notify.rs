//! Desktop notifications: what a chat tells the user while the window is
//! in the background (its agent finished, asks something, or stopped).

use crate::PrintCadApp;

/// The longest body shown, in characters.
const BODY_MAX: usize = 160;

impl PrintCadApp {
    /// Tell the user `summary` and `body`, when the window is in the
    /// background and the Preferences allow it.
    pub(crate) fn notify(&self, summary: String, body: &str) {
        if !self.user_settings.ai.notify || self.window_focused {
            return;
        }
        let body = excerpt(body);
        // The system's notification service can take a moment to answer.
        let _ = std::thread::Builder::new()
            .name("printcad-notify".to_string())
            .spawn(move || {
                let shown = notify_rust::Notification::new()
                    .appname("printCAD")
                    .summary(&summary)
                    .body(&body)
                    .icon("printcad")
                    .show();
                if let Err(err) = shown {
                    tracing::debug!(target: "printcad.notify", "no notification: {err}");
                }
            });
    }
}

/// `text` on one line, cut to [`BODY_MAX`].
fn excerpt(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match line.chars().count() > BODY_MAX {
        true => format!(
            "{}…",
            line.chars()
                .take(BODY_MAX - 1)
                .collect::<String>()
                .trim_end()
        ),
        false => line,
    }
}
