//! The running application as the UI names it: its version and its
//! release notes (`crates/app_shell/RELEASE_NOTES.md`: a `## <version>`
//! heading per release, `### <topic>` under it, and one bullet per
//! change), handed over once by the application at start.

use std::sync::OnceLock;

static BUILD: OnceLock<(&'static str, &'static str)> = OnceLock::new();

/// Name the running version and its release notes. The first call wins.
pub fn set(version: &'static str, release_notes: &'static str) {
    let _ = BUILD.set((version, release_notes));
}

/// The running version: empty until the application names it.
pub fn version() -> &'static str {
    BUILD.get().map_or("", |(version, _)| version)
}

/// The release notes' text: empty until the application names it.
pub fn release_notes() -> &'static str {
    BUILD.get().map_or("", |(_, notes)| notes)
}

#[derive(Debug, PartialEq)]
pub struct Release<'a> {
    pub version: &'a str,
    pub topics: Vec<Topic<'a>>,
}

#[derive(Debug, PartialEq)]
pub struct Topic<'a> {
    pub title: &'a str,
    pub changes: Vec<&'a str>,
}

/// The releases in `text`, in the order written (newest first). Lines
/// outside a release, and bullets outside a topic, are prose for whoever
/// edits the file and are left out.
pub fn parse(text: &str) -> Vec<Release<'_>> {
    let mut releases: Vec<Release<'_>> = Vec::new();
    for line in text.lines().map(str::trim_end) {
        if let Some(version) = line.strip_prefix("## ") {
            releases.push(Release {
                version: version.trim(),
                topics: Vec::new(),
            });
        } else if let Some(title) = line.strip_prefix("### ") {
            if let Some(release) = releases.last_mut() {
                release.topics.push(Topic {
                    title: title.trim(),
                    changes: Vec::new(),
                });
            }
        } else if let Some(change) = line.strip_prefix("- ")
            && let Some(topic) = releases.last_mut().and_then(|r| r.topics.last_mut())
        {
            topic.changes.push(change.trim());
        }
    }
    releases
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prose_outside_a_topic_is_left_out() {
        let text = "# Notes\n- not a change\n## 2.0\n- nor this\n### Fixes\n- a fix\n\n- another\n## 1.0\n### New\n- first\n";
        assert_eq!(
            parse(text),
            vec![
                Release {
                    version: "2.0",
                    topics: vec![Topic {
                        title: "Fixes",
                        changes: vec!["a fix", "another"],
                    }],
                },
                Release {
                    version: "1.0",
                    topics: vec![Topic {
                        title: "New",
                        changes: vec!["first"],
                    }],
                },
            ]
        );
    }
}
