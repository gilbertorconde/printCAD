//! A document's file kept safe: what wrote it and whether it can be
//! written in place, learnt when it opens, and the question asked before
//! saving over a file another version wrote.

use std::path::{Path, PathBuf};

use core_document::{Document, FileAge, file_age};
use uuid::Uuid;

use crate::Then;

/// What the tab knows of the file it is saved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSafety {
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
    /// What `document`, just read from `path`, and the file itself say,
    /// to a build of version `running`. A plain `.json` document is not
    /// a container and carries no stamp.
    pub fn of_opened(path: &Path, document: &Document, read_only: bool, running: &str) -> Self {
        let container = !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"));
        let age = if container {
            file_age(document.written_by(), running)
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
    pub fn writer_words(&self) -> String {
        match (&self.writer, self.age) {
            (Some(version), _) => format!("printCAD {version}"),
            (None, FileAge::Newer) => "a newer printCAD".to_string(),
            (None, _) => "an older printCAD".to_string(),
        }
    }

    /// Whether saving over the file asks first: always for a newer one,
    /// for an older one while `ask_older`.
    pub fn asks_before_saving(&self, ask_older: bool) -> bool {
        match self.age {
            FileAge::Newer => true,
            FileAge::Older => ask_older,
            FileAge::Same => false,
        }
    }

    /// What the card for a file from a newer printCAD says to a build of
    /// version `running`.
    pub fn newer_text(&self, running: &str) -> String {
        format!(
            "Written by {}, newer than this {running}: saving over it may lose what this \
             version does not know",
            self.writer_words()
        )
    }
}

/// What a read-only file's marks say.
pub const READ_ONLY_TIP: &str =
    "The file is read-only: it cannot be saved in place, and Save asks for another name";

/// What a read-only linked file's mark says.
pub const READ_ONLY_LINK_TIP: &str =
    "Its file is read-only: the part reads from it, but the file cannot be saved in place";

/// A save over a file from another version, waiting for the user.
#[derive(Debug, Clone)]
pub struct SaveOver {
    pub tab: Uuid,
    pub path: PathBuf,
    /// The close or quit that asked for the save, which goes on once the
    /// file is written.
    pub then: Option<Then>,
}

/// What answering the question about saving over a file leads to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SaveOverOutcome {
    /// The settings changed and want writing.
    pub settings_changed: bool,
    /// The close or quit waiting on the save, to go on with now that the
    /// file is written.
    pub go_on: Option<(Uuid, Then)>,
}

/// What the user chose about saving over a file from another version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOverAnswer {
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
pub struct SaveOverQuestion {
    pub name: String,
    pub writer: String,
    pub newer: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A build's version, as the app names its own.
    const RUNNING: &str = "0.18.0";

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
        let opened = FileSafety::of_opened(Path::new("a.prtcad"), &loaded, false, RUNNING);
        assert_eq!(opened.age, FileAge::Same);
        assert!(!opened.newer_notice);

        let fresh = core_document::Document::new("Plain");
        let opened = FileSafety::of_opened(Path::new("a.prtcad"), &fresh, true, RUNNING);
        assert_eq!(opened.age, FileAge::Older);
        assert!(opened.read_only);
        let opened = FileSafety::of_opened(Path::new("a.json"), &fresh, false, RUNNING);
        assert_eq!(opened.age, FileAge::Same);
    }
}
