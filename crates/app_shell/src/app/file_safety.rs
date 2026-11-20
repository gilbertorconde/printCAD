//! A document's file kept safe in the running app: learnt when it
//! opens. A file a newer printCAD wrote says so in a card offering Save
//! As; a read-only one is marked on its tab and in the tree, and Save asks
//! for another name; saving over a file from another version asks first,
//! for an older one only while Preferences › General says to.

use std::path::{Path, PathBuf};

use core_document::FileAge;
use file_session::{FileSafety, SaveOver, SaveOverAnswer, SaveOverOutcome, SaveOverQuestion, Then};
use uuid::Uuid;

use crate::PrintCadApp;

/// The version this build is, as saves are stamped with it.
pub(crate) const RUNNING: &str = env!("CARGO_PKG_VERSION");

impl PrintCadApp {
    /// Learn what wrote the file just opened into the active tab and
    /// whether it can be written, and say what the user should know.
    pub(crate) fn note_opened_file(&mut self, path: &Path) {
        let read_only = app_platform::read_only(path);
        let safety = FileSafety::of_opened(path, &self.session.document, read_only, RUNNING);
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
        self.session.file.safety = safety;
    }

    /// The file the tab is saved to was just written by this build.
    pub(crate) fn note_saved_file(&mut self) {
        self.session.file.safety = FileSafety::default();
    }

    /// Whether the tab's own file cannot be written in place, as it was
    /// found when opened or as it stands now.
    pub(crate) fn current_file_read_only(&self) -> bool {
        self.session.file.safety.read_only
            || self
                .session
                .file
                .path
                .as_deref()
                .is_some_and(app_platform::read_only)
    }

    /// Save to `path`, picked by Save or Save As: over the tab's own file
    /// from another version, once the user says so. With `then`, what
    /// waits on the save, it is waited for, and what goes on once it is
    /// written is given back; a save that fails stops it.
    pub(crate) fn save_document_asking(
        &mut self,
        path: PathBuf,
        then: Option<Then>,
    ) -> Option<(Uuid, Then)> {
        if self.asks_before_saving_at(&path) {
            self.questions.save_over = Some(SaveOver {
                tab: self.session.tab,
                path,
                then,
            });
            self.redraw_needed = true;
            return None;
        }
        let saved = match self.save_document_at(&path) {
            Ok(()) => then.is_none_or(|then| self.save_landed(then)),
            Err(err) => {
                app_log::error(format!("Failed to save document: {err:#}"));
                false
            }
        };
        match then {
            Some(then) if saved => Some((self.session.tab, then)),
            Some(_) => {
                self.questions.stop_waiting();
                None
            }
            None => None,
        }
    }

    /// Whether saving the active tab to `path` asks first: it is the tab's
    /// own file, from another version.
    pub(crate) fn asks_before_saving_at(&self, path: &Path) -> bool {
        self.session.file.path.as_deref() == Some(path)
            && self
                .session
                .file
                .safety
                .asks_before_saving(self.user_settings.files.ask_before_saving_older)
    }

    /// Save of the active tab in place, asked for by a close or quit
    /// (`then`): when its own file is from another version, put up the
    /// question about saving over it, which carries `then` on. True when
    /// the question is up.
    pub(crate) fn ask_before_saving_over(&mut self, then: Option<Then>) -> bool {
        let Some(path) = self.session.file.path.clone() else {
            return false;
        };
        if self.current_file_read_only() || !self.asks_before_saving_at(&path) {
            return false;
        }
        self.questions.save_over = Some(SaveOver {
            tab: self.session.tab,
            path,
            then,
        });
        self.redraw_needed = true;
        true
    }

    /// The question about saving over a file from another version, while
    /// it is open for the active tab.
    pub(crate) fn save_over_question(&self) -> Option<SaveOverQuestion> {
        let asking = self.questions.save_over.as_ref()?;
        if asking.tab != self.session.tab {
            return None;
        }
        let safety = &self.session.file.safety;
        Some(SaveOverQuestion {
            name: self.session.document.name().to_string(),
            writer: safety.writer_words(),
            newer: safety.age == FileAge::Newer,
        })
    }

    /// The user answered the question about saving over the file. A close
    /// or quit waiting on it goes on only once the file is written; Save
    /// As or Cancel stops it.
    pub(crate) fn answer_save_over(&mut self, answer: SaveOverAnswer) -> SaveOverOutcome {
        let mut outcome = SaveOverOutcome::default();
        let Some(asking) = self.questions.save_over.take() else {
            return outcome;
        };
        if asking.tab != self.session.tab {
            return outcome;
        }
        let mut saved = false;
        match answer {
            SaveOverAnswer::Save { quiet } => {
                match self.save_document_at(&asking.path) {
                    Ok(()) => saved = asking.then.is_none_or(|then| self.save_landed(then)),
                    Err(err) => app_log::error(format!("Failed to save document: {err:#}")),
                }
                if quiet && self.user_settings.files.ask_before_saving_older {
                    self.user_settings.files.ask_before_saving_older = false;
                    outcome.settings_changed = true;
                }
            }
            SaveOverAnswer::SaveAs => {
                self.start_file_dialog(crate::app::doc_io::FileDialogKind::SaveAs);
            }
            SaveOverAnswer::Cancel => {}
        }
        if let Some(then) = asking.then {
            if saved {
                outcome.go_on = Some((asking.tab, then));
            } else {
                self.questions.stop_waiting();
            }
        }
        outcome
    }
}
