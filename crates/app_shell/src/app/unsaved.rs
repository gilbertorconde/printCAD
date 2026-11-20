//! Closing a document with unsaved edits: the window asks (Save, Don't
//! save, Cancel) and the close waits for the answer. Quitting asks for each tab with edits in turn, then exits.
//! Save goes on only once the file is written: a save that fails keeps
//! the tab open, with its recovery copy, and stops the quit. A document
//! with no file of its own asks for a name in the file dialog, on its
//! thread, and the close goes on once the save under it lands.
//!
//! The same questions, one tab at a time, carry Close other tabs, Save
//! all (its names and saves over another version's file) and Revert to
//! saved, which asks before letting edits go.

use winit::event_loop::ActiveEventLoop;

use file_session::{Asking, Then, Unsaved};
use uuid::Uuid;

use crate::PrintCadApp;
use crate::app::doc_io::FileDialogKind;

impl PrintCadApp {
    /// The name of the document asked about while a question is open,
    /// and whether the question is Revert's.
    pub(crate) fn unsaved_question(&self) -> Option<(String, bool)> {
        let asking = self.questions.unsaved?;
        let session = self.tab_session(asking.tab)?;
        Some((
            session.document.name().to_string(),
            asking.then == Then::Revert,
        ))
    }

    fn is_dirty(&self, index: usize) -> bool {
        self.session_at(index)
            .is_some_and(|s| s.document.metadata().dirty())
    }

    /// Close tab `index`: at once when it has no unsaved edits, otherwise
    /// once the user answers. True when it closed now.
    pub(crate) fn close_tab_interactive(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }
        if !self.is_dirty(index) {
            self.close_tab_now(index);
            return true;
        }
        self.switch_tab(index);
        self.questions.unsaved = Some(Asking {
            tab: self.tabs[index].tab,
            then: Then::CloseTab,
        });
        self.redraw_needed = true;
        false
    }

    /// Quit: ask about each tab with unsaved edits, then exit.
    pub(crate) fn request_quit(&mut self, event_loop: &ActiveEventLoop) {
        let next = (0..self.tabs.len()).find(|&i| {
            self.is_dirty(i) && !self.questions.quit_answered.contains(&self.tabs[i].tab)
        });
        match next {
            Some(index) => {
                self.switch_tab(index);
                self.questions.unsaved = Some(Asking {
                    tab: self.tabs[index].tab,
                    then: Then::Quit,
                });
                self.redraw_needed = true;
            }
            None => {
                // Any save the answers started finishes before the exit;
                // leaving mid-write would truncate a file.
                self.wait_for_all_document_saves();
                // A save that failed on the way leaves its tab with edits
                // nobody answered for: that tab is asked about too.
                let unanswered = (0..self.tabs.len()).any(|i| {
                    self.is_dirty(i) && !self.questions.quit_answered.contains(&self.tabs[i].tab)
                });
                if unanswered {
                    self.request_quit(event_loop);
                    return;
                }
                // Every tab saved or let go: no copy to come back from.
                self.questions.stop_waiting();
                self.forget_recovery_copies(|_| true);
                self.leave_servers();
                self.keep_session_for_next_run();
                self.remove_private_folders();
                event_loop.exit();
            }
        }
    }

    /// Exit with the window unable to draw, so no question can be asked.
    pub(crate) fn exit_unable_to_draw(&mut self, event_loop: &ActiveEventLoop) {
        self.exit_without_asking(
            event_loop,
            "printCAD cannot draw its window and closes; unsaved documents are kept as \
             recovery copies, offered at the next start",
        );
    }

    /// Exit where no question can be answered, saying `why` in the log:
    /// every tab with edits keeps a fresh recovery copy, which the next
    /// start offers back, and the saves in flight finish first.
    pub(crate) fn exit_without_asking(&mut self, event_loop: &ActiveEventLoop, why: &str) {
        app_log::error(why);
        self.keep_recovery_copies();
        self.wait_for_all_document_saves();
        self.keep_session_for_next_run();
        self.remove_private_folders();
        event_loop.exit();
    }

    /// The user answered the open question.
    pub(crate) fn answer_unsaved(&mut self, answer: Unsaved, event_loop: &ActiveEventLoop) {
        let Some(asking) = self.questions.unsaved.take() else {
            return;
        };
        let Some(index) = self.tab_index_of(asking.tab) else {
            return;
        };
        let go_on = match answer {
            Unsaved::Cancel => false,
            Unsaved::Discard => true,
            Unsaved::Save => {
                self.switch_tab(index);
                if self.ask_before_saving_over(Some(asking.then)) {
                    // The question about the file's version answers for
                    // the close, and goes on with it once saved.
                    return;
                }
                match self.file_to_save_in_place() {
                    Some(path) => match self.save_document_at(&path) {
                        Ok(()) => self.save_landed(asking.then),
                        Err(err) => {
                            app_log::error(format!("Save failed: {err:#}"));
                            false
                        }
                    },
                    // The name is asked on the dialog's thread, the window
                    // drawing meanwhile; the close goes on once the
                    // document is saved under it.
                    None => {
                        self.ask_name_then(asking.then);
                        return;
                    }
                }
            }
        };
        if !go_on {
            self.questions.stop_waiting();
            return;
        }
        self.go_on_after_answer(asking.tab, asking.then, event_loop);
    }

    /// Ask the active tab's document a name in the file dialog, on its
    /// thread; `then` goes on once it is saved under it. Refused while
    /// another dialog is open, which stops `then` there.
    fn ask_name_then(&mut self, then: Then) {
        let refused = self.files.dialog_rx.is_some();
        self.start_file_dialog(FileDialogKind::SaveThen(then));
        if refused {
            self.questions.stop_waiting();
        }
    }

    /// What waited for the answer about tab `tab`'s edits: close it, go
    /// on quitting, closing the others or saving all, or revert it.
    pub(crate) fn go_on_after_answer(
        &mut self,
        tab: Uuid,
        then: Then,
        event_loop: &ActiveEventLoop,
    ) {
        match then {
            Then::CloseTab => {
                if let Some(index) = self.tab_index_of(tab) {
                    self.close_tab_now(index);
                }
            }
            Then::Quit => {
                self.questions.quit_answered.insert(tab);
                self.request_quit(event_loop);
            }
            Then::CloseOthers(keep) => {
                if let Some(index) = self.tab_index_of(tab) {
                    self.close_tab_now(index);
                }
                self.close_other_tabs(keep);
            }
            Then::SaveAll => self.save_all(),
            Then::Revert => self.revert_now(tab),
        }
    }

    /// Close every tab but `keep`: those without edits at once, then each
    /// with edits once the user answers for it, as quitting asks.
    pub(crate) fn close_other_tabs(&mut self, keep: Uuid) {
        while let Some(index) =
            (0..self.tabs.len()).find(|&i| self.tabs[i].tab != keep && !self.is_dirty(i))
        {
            self.close_tab_now(index);
        }
        match (0..self.tabs.len()).find(|&i| self.tabs[i].tab != keep) {
            Some(index) => {
                self.switch_tab(index);
                self.questions.unsaved = Some(Asking {
                    tab: self.tabs[index].tab,
                    then: Then::CloseOthers(keep),
                });
                self.redraw_needed = true;
            }
            None => {
                if let Some(index) = self.tab_index_of(keep) {
                    self.switch_tab(index);
                }
            }
        }
    }

    /// Save every tab with edits: each that saves in place at once, then
    /// one at a time each that asks first, for a name or about saving over
    /// another version's file, going on as each save lands.
    pub(crate) fn save_all(&mut self) {
        for index in 0..self.tabs.len() {
            let tab = self.tabs[index].tab;
            if !self.is_dirty(index) || self.questions.saved_all.contains(&tab) {
                continue;
            }
            let quiet = self.with_tab(index, |app| {
                app.file_to_save_in_place()
                    .filter(|path| !app.asks_before_saving_at(path))
            });
            if let Some(path) = quiet {
                self.questions.saved_all.insert(tab);
                self.with_tab(index, |app| {
                    if let Err(err) = app.save_document_at(&path) {
                        app_log::error(format!("Save failed: {err:#}"));
                    }
                });
            }
        }
        let next = (0..self.tabs.len())
            .find(|&i| self.is_dirty(i) && !self.questions.saved_all.contains(&self.tabs[i].tab));
        let Some(index) = next else {
            self.questions.saved_all.clear();
            return;
        };
        self.questions.saved_all.insert(self.tabs[index].tab);
        self.switch_tab(index);
        if !self.ask_before_saving_over(Some(Then::SaveAll)) {
            self.ask_name_then(Then::SaveAll);
        }
    }

    /// Revert tab `tab` to its file: at once when it has no edits, else
    /// once the user lets them go.
    pub(crate) fn revert_tab(&mut self, tab: Uuid) {
        let Some(index) = self.tab_index_of(tab) else {
            return;
        };
        if self.session_at(index).is_none_or(|s| s.file.path.is_none()) {
            app_log::warn("This document has no saved file to revert to");
            return;
        }
        if !self.is_dirty(index) {
            self.revert_now(tab);
            return;
        }
        self.switch_tab(index);
        self.questions.unsaved = Some(Asking {
            tab,
            then: Then::Revert,
        });
        self.redraw_needed = true;
    }

    /// Read tab `tab`'s file again in its place, whatever its edits: the
    /// tab closes and the file opens where it stood, as closing and
    /// opening it would.
    fn revert_now(&mut self, tab: Uuid) {
        let Some(index) = self.tab_index_of(tab) else {
            return;
        };
        let Some(path) = self.session_at(index).and_then(|s| s.file.path.clone()) else {
            return;
        };
        if !app_platform::exists(&path) {
            app_log::error(format!(
                "`{}` is gone: nothing to revert to",
                path.display()
            ));
            return;
        }
        self.switch_tab(index);
        // A save on its way is written before the file is read again.
        self.wait_for_document_saves(Then::Revert.after_save());
        self.close_tab_now(index);
        self.files.closed_files.retain(|f| f != &path);
        self.open_document_at(path);
        let opened = self.tabs[self.active_tab].tab;
        self.move_tab(opened, index);
    }

    /// Whether the save the active tab just started for `then` is
    /// written: it is waited for, its answer taken in (the file, the
    /// recent list), and the document is clean. Said in the log when it
    /// is not.
    pub(crate) fn save_landed(&mut self, then: Then) -> bool {
        self.wait_for_document_saves(then.after_save());
        self.drain_server_messages();
        let saved = !self.session.document.metadata().dirty();
        if !saved {
            app_log::error(format!(
                "`{}` was not saved, so it stays open with its edits",
                self.session.document.name()
            ));
        }
        saved
    }
}
