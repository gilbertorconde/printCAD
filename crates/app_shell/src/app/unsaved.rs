//! Closing a document with unsaved edits: the window asks (Save, Don't
//! save, Cancel; `ui/unsaved_modal.rs`) and the close waits for the
//! answer. Quitting asks for each tab with edits in turn, then exits.
//! Save goes on only once the file is written: a save that fails keeps
//! the tab open, with its recovery copy, and stops the quit.

use uuid::Uuid;
use winit::event_loop::ActiveEventLoop;

use crate::PrintCadApp;
use crate::log_panel as app_log;

/// What the user chose about unsaved edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unsaved {
    Save,
    Discard,
    Cancel,
}

/// What waits for the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Then {
    /// Close the tab asked about.
    CloseTab,
    /// Go on quitting: ask about the next tab with edits, or exit.
    Quit,
}

/// A question on screen: which tab, and what follows the answer.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Asking {
    pub tab: Uuid,
    pub then: Then,
}

impl PrintCadApp {
    /// The name of the document asked about, while a question is open.
    pub(crate) fn unsaved_question(&self) -> Option<String> {
        let asking = self.questions.unsaved?;
        let session = self.tab_session(asking.tab)?;
        Some(session.document.name().to_string())
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
                self.questions.quit_answered.clear();
                self.forget_recovery_copies(|_| true);
                self.keep_session_for_next_run();
                event_loop.exit();
            }
        }
    }

    /// Exit with the window unable to draw, so no question can be asked:
    /// every tab with edits keeps a fresh recovery copy, which the next
    /// start offers back, and the saves in flight finish first.
    pub(crate) fn exit_unable_to_draw(&mut self, event_loop: &ActiveEventLoop) {
        app_log::error(
            "printCAD cannot draw its window and closes; unsaved documents are kept as \
             recovery copies, offered at the next start",
        );
        self.keep_recovery_copies();
        self.wait_for_all_document_saves();
        self.keep_session_for_next_run();
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
                self.save_document_interactive() && self.save_landed()
            }
        };
        if !go_on {
            self.questions.quit_answered.clear();
            return;
        }
        self.go_on_after_answer(asking.tab, asking.then, event_loop);
    }

    /// What waited for the answer about tab `tab`'s edits: close it, or
    /// go on quitting.
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
        }
    }

    /// Whether the save the active tab just started is written: it is
    /// waited for, its answer taken in (the file, the recent list), and
    /// the document is clean. Said in the log when it is not.
    pub(crate) fn save_landed(&mut self) -> bool {
        self.wait_for_document_saves();
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
