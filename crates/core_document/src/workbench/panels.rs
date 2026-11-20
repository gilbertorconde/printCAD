//! The task panel, floating card, menus and settings a bench offers.

use super::{MenuItem, MenuScope, TaskInfo};
use crate::Document;
use crate::runtime::WorkbenchRuntimeContext;

/// The chrome a bench fills with declared data the host draws: the task
/// panel's state, the floating card, contextual menu entries and the
/// bench's own settings. What the bench draws itself with egui is its
/// drawing side's.
pub trait BenchPanels {
    /// What this bench offers in a contextual menu, computed while the
    /// menu is open. Every bench is asked, whichever is active.
    fn menu_items(&self, _scope: &MenuScope, _document: &Document) -> Vec<MenuItem> {
        Vec::new()
    }

    /// Run one of this bench's `menu_items` ids in the scope it was
    /// offered in. `false` when the id is not one of this bench's.
    fn on_command(
        &mut self,
        _id: &str,
        _scope: &MenuScope,
        _ctx: &mut WorkbenchRuntimeContext,
    ) -> bool {
        false
    }

    /// The card over the view next to what it edits, while the bench is
    /// active and has one: declared widgets the host draws as it draws a
    /// task panel's, with OK and Cancel. Only for a quick edit of a value
    /// or two (a dimension, a corner's size); a feature with more to set
    /// is a task. Asked every frame; edits wait in the bench until OK.
    fn floating_card(&mut self, _ctx: &WorkbenchRuntimeContext) -> Option<panel_api::FloatingCard> {
        None
    }

    /// A change on the floating card: its widgets' events, and its OK or
    /// Cancel as [`panel_api::PanelEvent::CardClose`], after which the
    /// bench puts the card away. OK writes what the card holds; the host
    /// closes it as one undo step, and on Cancel takes back anything
    /// written while the card was open.
    fn card_event(&mut self, _ctx: &mut WorkbenchRuntimeContext, _event: &panel_api::PanelEvent) {}

    /// What the task panel is editing, if anything. `Some` opens the panel.
    fn task(&self, _ctx: &WorkbenchRuntimeContext) -> Option<TaskInfo> {
        None
    }

    /// The bench has a Preferences page, which its drawing side draws:
    /// without one it takes no place in the rail.
    fn has_settings(&self) -> bool {
        false
    }

    /// This bench's own settings, for the host to keep in the user's
    /// settings file without reading them. `None` when it has none.
    fn settings_json(&self) -> Option<serde_json::Value> {
        None
    }

    /// Settings `settings_json` produced earlier, back from the file.
    fn apply_settings_json(&mut self, _value: &serde_json::Value) {}
}
