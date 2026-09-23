//! The release notes bundled with the app, shown by the start page's
//! What's new. `RELEASE_NOTES.md` at the crate root is the source: a
//! `## <version>` heading per release, `### <topic>` under it, and one
//! bullet per change.

use egui::{RichText, Ui};
use ui_kit::tokens::*;
use ui_kit::widgets::{Card, overline};
use ui_kit::{sans, sans_semibold};

const SOURCE: &str = include_str!("../../RELEASE_NOTES.md");

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

/// The bundled notes, the running version first.
pub fn bundled() -> Vec<Release<'static>> {
    let mut releases = parse(SOURCE);
    let running = env!("CARGO_PKG_VERSION");
    if let Some(at) = releases.iter().position(|r| r.version == running) {
        let current = releases.remove(at);
        releases.insert(0, current);
    }
    releases
}

/// Every release as a card of topics.
pub fn draw(ui: &mut Ui) {
    let running = env!("CARGO_PKG_VERSION");
    for release in bundled() {
        let heading = if release.version == running {
            format!("printCAD {} · this version", release.version)
        } else {
            format!("printCAD {}", release.version)
        };
        ui.label(
            RichText::new(heading)
                .font(sans_semibold(FONT_LG))
                .color(TEXT1),
        );
        Card::new().padding(SPACE_4).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for topic in &release.topics {
                overline(ui, topic.title);
                for change in &topic.changes {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("•").font(sans(FONT_SM)).color(ACCENT));
                        ui.label(RichText::new(*change).font(sans(FONT_SM)).color(TEXT2));
                    });
                }
                ui.add_space(SPACE_2);
            }
        });
        ui.add_space(SPACE_4);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_running_version_has_notes_and_every_topic_says_something() {
        let releases = bundled();
        assert_eq!(releases[0].version, env!("CARGO_PKG_VERSION"));
        for release in &releases {
            assert!(
                !release.topics.is_empty(),
                "{} has no topics",
                release.version
            );
            for topic in &release.topics {
                assert!(
                    !topic.changes.is_empty(),
                    "{} › {} is empty",
                    release.version,
                    topic.title
                );
            }
        }
    }

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
