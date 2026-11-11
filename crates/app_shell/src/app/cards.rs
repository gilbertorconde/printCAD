//! The active bench's floating card as the host keeps it: the undo step
//! before it closed as it opens, its edits one step while it is open, OK
//! closing that step and Cancel taking it back, so a cancelled card leaves
//! the document as it found it.

use core_document::WorkbenchId;

use super::workbench_host::HookSite;
use crate::PrintCadApp;

/// A floating card on screen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OpenCard {
    pub id: String,
    pub bench: WorkbenchId,
    /// OK is held back: Enter and a click in the view do nothing.
    pub refused: bool,
}

impl PrintCadApp {
    /// Follow the card the UI drew this frame: one that opens closes the
    /// undo step before it, so the card's edits are a step of their own.
    pub(crate) fn follow_card(&mut self, shown: Option<(String, bool)>) {
        let Some((id, refused)) = shown else {
            self.session.card = None;
            return;
        };
        let bench = self.session.active_workbench.0.clone();
        match self.session.card.as_mut() {
            Some(card) if card.id == id && card.bench == bench => card.refused = refused,
            _ => {
                self.close_gesture();
                self.session.card = Some(OpenCard { id, bench, refused });
            }
        }
    }

    /// Close the card: the bench hears `CardClose`, then what it wrote
    /// while open is one undo step on OK, and taken back on Cancel.
    /// Answers whether it closed: OK is refused while held back.
    pub(crate) fn close_card(&mut self, accept: bool) -> bool {
        let Some(card) = self.session.card.take() else {
            return false;
        };
        if accept && card.refused {
            self.session.card = Some(card);
            return false;
        }
        let event = bench_api::PanelEvent::CardClose {
            id: card.id.clone(),
            accept,
        };
        let params = self.interaction_ctx_params();
        if let Some(((), outcome)) =
            self.with_workbench_ctx(&card.bench, params, |wb, ctx| wb.card_event(ctx, &event))
        {
            self.apply_hook_outcome(outcome, HookSite::Interaction);
        }
        let written = self.close_gesture();
        if !accept && let Some(entry) = written {
            self.tool_undo(entry);
        }
        self.redraw_needed = true;
        true
    }
}
