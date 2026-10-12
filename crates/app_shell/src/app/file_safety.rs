//! A document's file kept safe: what wrote it and whether it can be
//! written in place, learnt when it opens. A file a newer printCAD wrote
//! says so in a card offering Save As; a read-only one is marked on its
//! tab and in the tree, and Save asks for another name; saving over a
//! file from another version asks first (`ui/save_over_modal.rs`), for an
//! older one only while Preferences › General says to.

use std::path::{Path, PathBuf};

use core_document::{Document, FileAge, file_age};
use uuid::Uuid;

use crate::PrintCadApp;
use crate::log_panel as app_log;

/// The version this build is, as saves are stamped with it.
const RUNNING: &str = env!("CARGO_PKG_VERSION");

/// What the tab knows of the file it is saved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileSafety {
    /// How the file's writer compares with this build.
    pub age: FileAge,
    /// The version that wrote it, when the file says.
    pub writer: Option<String>,
    /// The file cannot be written in place: Save asks for another name.
    pub read_only: bool,
    /// The card saying a newer printCAD wrote the file is up.
    pub newer_notice: bool,
}

impl Default for FileSafety {
    /// A file this build wrote, or none at all.
    fn default() -> Self {
        Self {
            age: FileAge::Same,
            writer: None,
            read_only: false,
            newer_notice: false,
        }
    }
}

impl FileSafety {
    /// What `document`, just read from `path`, and the file itself say.
    /// A plain `.json` document is not a container and carries no stamp.
    pub(crate) fn of_opened(path: &Path, document: &Document, read_only: bool) -> Self {
        let container = !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"));
        let age = if container {
            file_age(document.written_by(), RUNNING)
        } else {
            FileAge::Same
        };
        Self {
            age,
            writer: document
                .written_by()
                .map(|s| s.app_version.clone())
                .filter(|v| !v.is_empty()),
            read_only,
            newer_notice: age == FileAge::Newer,
        }
    }

    /// "printCAD 0.9.0", or what is known of the version without its
    /// number.
    pub(crate) fn writer_words(&self) -> String {
        match (&self.writer, self.age) {
            (Some(version), _) => format!("printCAD {version}"),
            (None, FileAge::Newer) => "a newer printCAD".to_string(),
            (None, _) => "an older printCAD".to_string(),
        }
    }

    /// Whether saving over the file asks first: always for a newer one,
    /// for an older one while `ask_older`.
    pub(crate) fn asks_before_saving(&self, ask_older: bool) -> bool {
        match self.age {
            FileAge::Newer => true,
            FileAge::Older => ask_older,
            FileAge::Same => false,
        }
    }

    /// What the card for a file from a newer printCAD says.
    pub(crate) fn newer_text(&self) -> String {
        format!(
            "Written by {}, newer than this {RUNNING}: saving over it may lose what this \
             version does not know",
            self.writer_words()
        )
    }
}

/// What a read-only file's marks say.
pub(crate) const READ_ONLY_TIP: &str =
    "The file is read-only: it cannot be saved in place, and Save asks for another name";

/// What a read-only linked file's mark says.
pub(crate) const READ_ONLY_LINK_TIP: &str =
    "Its file is read-only: the part reads from it, but the file cannot be saved in place";

/// A save over a file from another version, waiting for the user.
#[derive(Debug, Clone)]
pub(crate) struct SaveOver {
    pub tab: Uuid,
    pub path: PathBuf,
}

/// What the user chose about saving over a file from another version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SaveOverAnswer {
    /// Save over it; `quiet` stops asking for older files.
    Save {
        quiet: bool,
    },
    SaveAs,
    Cancel,
}

/// The question on screen: the document's name, what wrote the file, and
/// whether that was a newer version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SaveOverQuestion {
    pub name: String,
    pub writer: String,
    pub newer: bool,
}

impl PrintCadApp {
    /// Learn what wrote the file just opened into the active tab and
    /// whether it can be written, and say what the user should know.
    pub(crate) fn note_opened_file(&mut self, path: &Path) {
        let read_only = crate::platform::read_only(path);
        let safety = FileSafety::of_opened(path, &self.session.document, read_only);
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into(),
        );
        if safety.read_only {
            app_log::warn(format!(
                "{name} is read-only: it cannot be saved in place, and Save asks for another name"
            ));
        }
        if safety.age == FileAge::Newer {
            app_log::warn(format!(
                "{name} was written by {}, newer than this {RUNNING}: saving over it may lose \
                 what this version does not know; Save As keeps it",
                safety.writer_words()
            ));
        }
        self.session.file_safety = safety;
    }

    /// The file the tab is saved to was just written by this build.
    pub(crate) fn note_saved_file(&mut self) {
        self.session.file_safety = FileSafety::default();
    }

    /// Whether the tab's own file cannot be written in place, as it was
    /// found when opened or as it stands now.
    pub(crate) fn current_file_read_only(&self) -> bool {
        self.session.file_safety.read_only
            || self
                .session
                .current_file
                .as_deref()
                .is_some_and(crate::platform::read_only)
    }

    /// Save to `path`, picked by Save or Save As: over the tab's own file
    /// from another version, once the user says so.
    pub(crate) fn save_document_asking(&mut self, path: PathBuf) {
        let own_file = self.session.current_file.as_deref() == Some(path.as_path());
        if own_file
            && self
                .session
                .file_safety
                .asks_before_saving(self.user_settings.files.ask_before_saving_older)
        {
            self.save_over_asking = Some(SaveOver {
                tab: self.session.tab,
                path,
            });
            self.redraw_needed = true;
            return;
        }
        if let Err(err) = self.save_document_at(&path) {
            app_log::error(format!("Failed to save document: {err:#}"));
        }
    }

    /// The question about saving over a file from another version, while
    /// it is open for the active tab.
    pub(crate) fn save_over_question(&self) -> Option<SaveOverQuestion> {
        let asking = self.save_over_asking.as_ref()?;
        if asking.tab != self.session.tab {
            return None;
        }
        let safety = &self.session.file_safety;
        Some(SaveOverQuestion {
            name: self.session.document.name().to_string(),
            writer: safety.writer_words(),
            newer: safety.age == FileAge::Newer,
        })
    }

    /// The user answered the question about saving over the file.
    /// Whether the settings changed and want writing.
    pub(crate) fn answer_save_over(&mut self, answer: SaveOverAnswer) -> bool {
        let Some(asking) = self.save_over_asking.take() else {
            return false;
        };
        if asking.tab != self.session.tab {
            return false;
        }
        match answer {
            SaveOverAnswer::Save { quiet } => {
                if let Err(err) = self.save_document_at(&asking.path) {
                    app_log::error(format!("Failed to save document: {err:#}"));
                }
                if quiet && self.user_settings.files.ask_before_saving_older {
                    self.user_settings.files.ask_before_saving_older = false;
                    return true;
                }
                false
            }
            SaveOverAnswer::SaveAs => {
                self.start_file_dialog(crate::app::doc_io::FileDialogKind::SaveAs);
                false
            }
            SaveOverAnswer::Cancel => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn safety(age: FileAge, writer: Option<&str>) -> FileSafety {
        FileSafety {
            age,
            writer: writer.map(str::to_string),
            ..FileSafety::default()
        }
    }

    #[test]
    fn a_newer_file_always_asks_and_an_older_one_as_the_setting_says() {
        assert!(safety(FileAge::Newer, None).asks_before_saving(false));
        assert!(safety(FileAge::Older, None).asks_before_saving(true));
        assert!(!safety(FileAge::Older, None).asks_before_saving(false));
        assert!(!safety(FileAge::Same, None).asks_before_saving(true));
    }

    #[test]
    fn the_writer_is_named_by_its_version_when_the_file_says() {
        assert_eq!(
            safety(FileAge::Newer, Some("9.0.0")).writer_words(),
            "printCAD 9.0.0"
        );
        assert_eq!(
            safety(FileAge::Newer, None).writer_words(),
            "a newer printCAD"
        );
        assert_eq!(
            safety(FileAge::Older, None).writer_words(),
            "an older printCAD"
        );
    }

    /// A document read from a container takes its age from the stamp; a
    /// plain JSON document has none to take and reads as this version's.
    #[test]
    fn an_opened_file_reads_its_age_from_the_stamp() {
        let mut stamped = core_document::Document::new("Old");
        let bytes = stamped
            .save_to_bytes(core_document::Compression::None)
            .expect("save");
        let loaded = core_document::Document::load_from_bytes(bytes).expect("load");
        // Saved by a build that named no version: the format alone, which
        // is this build's.
        let opened = FileSafety::of_opened(Path::new("a.prtcad"), &loaded, false);
        assert_eq!(opened.age, FileAge::Same);
        assert!(!opened.newer_notice);

        let fresh = core_document::Document::new("Plain");
        let opened = FileSafety::of_opened(Path::new("a.prtcad"), &fresh, true);
        assert_eq!(opened.age, FileAge::Older);
        assert!(opened.read_only);
        let opened = FileSafety::of_opened(Path::new("a.json"), &fresh, false);
        assert_eq!(opened.age, FileAge::Same);
    }
}
