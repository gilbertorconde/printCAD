//! The question about unsaved edits: its answers and what waits for
//! one.

use uuid::Uuid;

/// What the user chose about unsaved edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unsaved {
    Save,
    Discard,
    Cancel,
}

/// What waits for the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Then {
    /// Close the tab asked about.
    CloseTab,
    /// Go on quitting: ask about the next tab with edits, or exit.
    Quit,
    /// Close the tab asked about, then go on closing every tab but this
    /// one.
    CloseOthers(Uuid),
    /// Go on saving every tab with edits.
    SaveAll,
    /// Read the tab's file again, its edits let go.
    Revert,
}

impl Then {
    /// What goes on once a save waited on lands, as the log's
    /// "Finishing document save before …" says it.
    pub fn after_save(self) -> &'static str {
        match self {
            Then::CloseTab | Then::CloseOthers(_) => "closing the tab",
            Then::Quit => "exit",
            Then::SaveAll => "saving the next tab",
            Then::Revert => "reading the file again",
        }
    }
}

/// A question on screen: which tab, and what follows the answer.
#[derive(Debug, Clone, Copy)]
pub struct Asking {
    pub tab: Uuid,
    pub then: Then,
}
