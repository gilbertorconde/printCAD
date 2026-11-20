//! Whether a newer printCAD is out, looked for on a package thread when
//! the app starts (when the user allows it) and whenever asked.

use package_store::{PackageNews, ReleaseCheck};

use crate::PrintCadApp;

impl PrintCadApp {
    /// Look for a newer release, away from the window. `quiet` says only
    /// a newer release in the log, as the look at start does.
    pub(crate) fn check_app_release(&mut self, quiet: bool) {
        if self.packages.release_check == ReleaseCheck::Checking {
            return;
        }
        self.packages.release_check = ReleaseCheck::Checking;
        self.package_thread(move || {
            let found = package_store::latest_release(env!("CARGO_PKG_VERSION"));
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
