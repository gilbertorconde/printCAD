//! Whether a newer printCAD is out: the latest release on GitHub. Nothing
//! is downloaded; the Updates page says what was found.

/// Where the app's releases are published.
pub const REPOSITORY: &str = "gilbertorconde/printCAD";
pub const RELEASES_PAGE: &str = "https://github.com/gilbertorconde/printCAD/releases";

/// What the last look at the releases found.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum ReleaseCheck {
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

/// Look at the latest release, `current` being this build's version.
/// Slow: it asks the network.
pub fn latest_release(current: &str) -> ReleaseCheck {
    workbenches::latest_release(REPOSITORY, current)
        .map(|(latest, newer)| ReleaseCheck::Found {
            tag: latest.tag,
            page: latest.page,
            newer,
        })
        .unwrap_or_else(ReleaseCheck::Failed)
}
