//! The tab strip's model: which sessions are open, which is on screen,
//! and how a background tab gets a turn.

use std::path::Path;

use uuid::Uuid;

use crate::PrintCadApp;
use crate::app::session::{DocumentSession, TabSlot};
use crate::log_panel as app_log;
use crate::ui::{Screen, TabInfo};

impl PrintCadApp {
    /// A fresh Untitled session, as a new tab would start.
    pub(crate) fn new_session(&self, screen: Screen) -> DocumentSession {
        DocumentSession::untitled(&self.user_settings.camera, self.landing_workbench(), screen)
    }

    /// The strip, in order, as the UI draws it.
    pub(crate) fn tab_infos(&self) -> Vec<TabInfo> {
        self.tabs
            .iter()
            .map(|slot| {
                let session = slot.parked.as_ref().unwrap_or(&self.session);
                TabInfo {
                    tab: slot.tab,
                    name: session.document.name().to_string(),
                    dirty: session.document.metadata().dirty(),
                    active: slot.tab == self.session.tab,
                    blank: self.tab_is_blank(session),
                }
            })
            .collect()
    }

    /// Nothing has happened in the tab, and nothing is on its way to it:
    /// a STEP import in flight is the kernel worker's to track, so the
    /// session alone cannot tell.
    fn tab_is_blank(&self, session: &DocumentSession) -> bool {
        session.is_blank() && !self.import_owner.values().any(|tab| *tab == session.tab)
    }

    pub(crate) fn tab_index_of(&self, tab: Uuid) -> Option<usize> {
        self.tabs.iter().position(|slot| slot.tab == tab)
    }

    /// The tab whose document has this body, wherever it is parked.
    pub(crate) fn tab_index_of_body(&self, body: Uuid) -> Option<usize> {
        self.tabs.iter().position(|slot| {
            let session = slot.parked.as_ref().unwrap_or(&self.session);
            session.document.bodies().iter().any(|b| b.id.0 == body)
        })
    }

    /// The tab that has this file open.
    pub(crate) fn tab_index_of_file(&self, path: &Path) -> Option<usize> {
        self.tabs.iter().position(|slot| {
            let session = slot.parked.as_ref().unwrap_or(&self.session);
            session.current_file.as_deref() == Some(path)
        })
    }

    /// Whether any tab has work on its way through a worker or the server.
    pub(crate) fn any_tab_busy(&self) -> bool {
        tabs_busy(&self.session, &self.tabs)
    }

    /// Put `session` on screen as a new tab after the active one.
    pub(crate) fn open_tab(&mut self, session: DocumentSession) {
        self.session.bench_states = self.registry.suspend_sessions();
        let outgoing = std::mem::replace(&mut self.session, session);
        self.carry_viewport_from(&outgoing);
        let at = self.active_tab + 1;
        self.tabs[self.active_tab].parked = Some(outgoing);
        self.tabs.insert(
            at,
            TabSlot {
                tab: self.session.tab,
                parked: None,
            },
        );
        self.active_tab = at;
        self.registry
            .resume_sessions(std::mem::take(&mut self.session.bench_states));
        self.redraw_needed = true;
    }

    /// Bring tab `index` on screen.
    pub(crate) fn switch_tab(&mut self, index: usize) {
        if index == self.active_tab || index >= self.tabs.len() {
            return;
        }
        let Some(incoming) = self.tabs[index].parked.take() else {
            return;
        };
        self.session.bench_states = self.registry.suspend_sessions();
        let outgoing = std::mem::replace(&mut self.session, incoming);
        self.carry_viewport_from(&outgoing);
        self.tabs[self.active_tab].parked = Some(outgoing);
        self.active_tab = index;
        self.registry
            .resume_sessions(std::mem::take(&mut self.session.bench_states));
        self.redraw_needed = true;
    }

    /// Bring the tab after (or before) the active one on screen, wrapping.
    pub(crate) fn cycle_tab(&mut self, delta: i32) {
        self.switch_tab(cycled(self.active_tab, delta, self.tabs.len()));
    }

    /// The active tab or a blank one: New and Open reuse a tab nothing has
    /// happened in, and open another next to the active one otherwise.
    pub(crate) fn ensure_fresh_tab(&mut self) {
        if !self.tab_is_blank(&self.session) {
            let session = self.new_session(Screen::Workspace);
            self.open_tab(session);
        }
    }

    /// Close tab `index`, saving or discarding its edits as the user
    /// decides. `false` when they cancel. The last tab closing leaves a
    /// blank one behind.
    pub(crate) fn close_tab_interactive(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }
        let was_active = self.active_tab;
        self.switch_tab(index);
        if !self.confirm_discard_or_save() {
            self.switch_tab(was_active);
            return false;
        }
        // A save the dialog started is still being written; the
        // connection must not go away under it.
        self.wait_for_document_saves();
        self.close_active_tab();
        // Closing a background tab keeps the one that was on screen.
        if was_active != index {
            let back = if was_active > index {
                was_active - 1
            } else {
                was_active
            };
            self.switch_tab(back);
        }
        true
    }

    /// Drop the tab on screen and bring a neighbour on, or a blank tab
    /// when it was the last.
    fn close_active_tab(&mut self) {
        let index = self.active_tab;
        let replacement = neighbour_of(index, self.tabs.len())
            .and_then(|neighbour| self.tabs[neighbour].parked.take());
        let replacement = replacement.unwrap_or_else(|| self.new_session(Screen::Start));
        let mut closing = std::mem::replace(&mut self.session, replacement);
        self.carry_viewport_from(&closing);
        app_log::info(format!("Closed `{}`", closing.document.name()));
        closing.server.flush();
        drop(closing);
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.tabs.push(TabSlot {
                tab: self.session.tab,
                parked: None,
            });
            self.active_tab = 0;
        } else {
            self.active_tab = index.saturating_sub(1);
        }
        self.registry
            .resume_sessions(std::mem::take(&mut self.session.bench_states));
        self.redraw_needed = true;
    }

    /// Every open tab's unsaved edits, each saved or discarded as the user
    /// decides. `false` when they cancel on any of them.
    pub(crate) fn confirm_close_all(&mut self) -> bool {
        let was_active = self.active_tab;
        for index in 0..self.tabs.len() {
            let dirty = self.tabs[index]
                .parked
                .as_ref()
                .unwrap_or(&self.session)
                .document
                .metadata()
                .dirty();
            if !dirty {
                continue;
            }
            self.switch_tab(index);
            if !self.confirm_discard_or_save() {
                self.switch_tab(was_active);
                return false;
            }
        }
        self.switch_tab(was_active);
        true
    }

    /// Run `f` with tab `index` on screen, then put things back. The
    /// benches are not switched: `f` must not touch their editing state.
    pub(crate) fn with_tab<R>(&mut self, index: usize, f: impl FnOnce(&mut Self) -> R) -> R {
        if index == self.active_tab || index >= self.tabs.len() {
            return f(self);
        }
        let Some(other) = self.tabs[index].parked.take() else {
            return f(self);
        };
        let active = self.active_tab;
        let mine = std::mem::replace(&mut self.session, other);
        self.tabs[active].parked = Some(mine);
        self.active_tab = index;
        let result = f(self);
        let other = std::mem::replace(
            &mut self.session,
            self.tabs[active]
                .parked
                .take()
                .expect("the active session parks while another has its turn"),
        );
        self.tabs[index].parked = Some(other);
        self.active_tab = active;
        result
    }

    /// `f` once per tab, each on screen for its turn. Same rule as
    /// `with_tab`: no bench state.
    pub(crate) fn for_each_tab(&mut self, mut f: impl FnMut(&mut Self)) {
        for index in 0..self.tabs.len() {
            self.with_tab(index, &mut f);
        }
    }

    /// The viewport is the window's, not the tab's: a session coming on
    /// screen takes the outgoing one's.
    fn carry_viewport_from(&mut self, outgoing: &DocumentSession) {
        let vp = outgoing.camera.viewport_info();
        self.session
            .camera
            .update_viewport((vp.0 as u32, vp.1 as u32), (vp.2, vp.3));
    }
}

/// The tab `delta` steps from `active`, wrapping around the strip.
fn cycled(active: usize, delta: i32, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (active as i32 + delta).rem_euclid(len as i32) as usize
}

/// The tab that comes on screen when `index` closes: the one before it,
/// or the one after when it was first. `None` when it was the only tab.
fn neighbour_of(index: usize, len: usize) -> Option<usize> {
    if len < 2 {
        None
    } else if index > 0 {
        Some(index - 1)
    } else {
        Some(1)
    }
}

/// Whether the tab on screen or any parked one has work on its way. A
/// free function so the frame's end can ask while it holds the window.
pub(crate) fn tabs_busy(session: &DocumentSession, tabs: &[TabSlot]) -> bool {
    session.busy()
        || tabs
            .iter()
            .any(|slot| slot.parked.as_ref().is_some_and(|s| s.busy()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycling_wraps_both_ways_and_stays_put_alone() {
        assert_eq!(cycled(0, 1, 3), 1);
        assert_eq!(cycled(2, 1, 3), 0);
        assert_eq!(cycled(0, -1, 3), 2);
        assert_eq!(cycled(0, 1, 1), 0);
        assert_eq!(cycled(0, 1, 0), 0);
    }

    #[test]
    fn a_closing_tab_hands_over_to_the_one_before_it_or_the_one_after() {
        assert_eq!(neighbour_of(2, 3), Some(1));
        assert_eq!(neighbour_of(0, 3), Some(1));
        assert_eq!(neighbour_of(0, 1), None);
    }
}
