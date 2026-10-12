//! Closing a document with unsaved edits: the window asks (Save, Don't
//! save, Cancel; `ui/unsaved_modal.rs`) and the close waits for the
//! answer. Quitting asks for each tab with edits in turn, then exits.

use uuid::Uuid;
use winit::event_loop::ActiveEventLoop;

use crate::PrintCadApp;

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
        let asking = self.unsaved_asking?;
        let index = self.tab_index_of(asking.tab)?;
        let session = if index == self.active_tab {
            &self.session
        } else {
            self.tabs[index].parked.as_ref()?
        };
        Some(session.document.name().to_string())
    }

    fn is_dirty(&self, index: usize) -> bool {
        self.tabs[index]
            .parked
            .as_ref()
            .unwrap_or(&self.session)
            .document
            .metadata()
            .dirty()
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
        self.unsaved_asking = Some(Asking {
            tab: self.tabs[index].tab,
            then: Then::CloseTab,
        });
        self.redraw_needed = true;
        false
    }

    /// Quit: ask about each tab with unsaved edits, then exit.
    pub(crate) fn request_quit(&mut self, event_loop: &ActiveEventLoop) {
        let next = (0..self.tabs.len())
            .find(|&i| self.is_dirty(i) && !self.quit_answered.contains(&self.tabs[i].tab));
        match next {
            Some(index) => {
                self.switch_tab(index);
                self.unsaved_asking = Some(Asking {
                    tab: self.tabs[index].tab,
                    then: Then::Quit,
                });
                self.redraw_needed = true;
            }
            None => {
                // Any save the answers started finishes before the exit;
                // leaving mid-write would truncate a file.
                self.wait_for_all_document_saves();
                self.keep_session_for_next_run();
                event_loop.exit();
            }
        }
    }

    /// The user answered the open question.
    pub(crate) fn answer_unsaved(&mut self, answer: Unsaved, event_loop: &ActiveEventLoop) {
        let Some(asking) = self.unsaved_asking.take() else {
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
                let saved = self.save_document_interactive();
                if saved {
                    // The write finishes, and its answer is taken in (the
                    // file, the recent list), before the tab goes.
                    self.wait_for_document_saves();
                    self.drain_server_messages();
                }
                saved
            }
        };
        if !go_on {
            self.quit_answered.clear();
            return;
        }
        match asking.then {
            Then::CloseTab => {
                if let Some(index) = self.tab_index_of(asking.tab) {
                    self.close_tab_now(index);
                }
            }
            Then::Quit => {
                self.quit_answered.insert(asking.tab);
                self.request_quit(event_loop);
            }
        }
    }
}
