//! Whether a newer printCAD is out: the latest release on GitHub, read on
//! a thread when the app starts (when the user allows it) and whenever
//! asked. Nothing is downloaded; the Updates page says what was found.

use crate::PrintCadApp;
use crate::app::packages::PackageNews;
use crate::log_panel as app_log;

/// Where the app's releases are published.
pub(crate) const REPOSITORY: &str = "gilbertorconde/printCAD";
pub(crate) const RELEASES_PAGE: &str = "https://github.com/gilbertorconde/printCAD/releases";

/// What the last look at the releases found.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) enum ReleaseCheck {
    #[default]
    NotChecked,
    Checking,
    /// The latest release, its page, and whether it is newer than this
    /// build.
    Found {
        tag: String,
        page: String,
        newer: bool,
    },
    Failed(String),
}

impl PrintCadApp {
    /// Look for a newer release, away from the window. `quiet` says only
    /// a newer release in the log, as the look at start does.
    pub(crate) fn check_app_release(&mut self, quiet: bool) {
        if self.packages.release_check == ReleaseCheck::Checking {
            return;
        }
        self.packages.release_check = ReleaseCheck::Checking;
        self.package_thread(move || {
            let found = workbenches::latest_release(REPOSITORY, env!("CARGO_PKG_VERSION"))
                .map(|(latest, newer)| ReleaseCheck::Found {
                    tag: latest.tag,
                    page: latest.page,
                    newer,
                })
                .unwrap_or_else(ReleaseCheck::Failed);
            PackageNews::AppRelease { found, quiet }
        });
    }

    pub(crate) fn take_app_release(&mut self, found: ReleaseCheck, quiet: bool) {
        match &found {
            // The notice card says it; the log keeps it quietly.
            ReleaseCheck::Found {
                tag, newer: true, ..
            } => app_log::info(format!("printCAD {tag} is out")),
            ReleaseCheck::Found { tag, .. } if !quiet => {
                app_log::info(format!("printCAD is up to date (latest release {tag})"));
            }
            ReleaseCheck::Failed(e) if !quiet => {
                app_log::warn(format!("Could not look for updates: {e}"));
            }
            _ => {}
        }
        self.packages.release_check = found;
    }
}
