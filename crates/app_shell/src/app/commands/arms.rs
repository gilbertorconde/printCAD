//! What a phase-1 command does when it acts at once and takes more than a
//! line: the view's menu entries, visibility and isolation, the shape work
//! a body asks the kernel for, and small edits of the tab's tools.

use crate::PrintCadApp;
use crate::log_panel as app_log;

/// Work a body asks the kernel for, each a history barrier.
pub(super) enum ShapeWork {
    Convert,
    Refine,
    Repair,
}

impl PrintCadApp {
    /// The export dialog, set to write the print layout.
    pub(super) fn open_export_laid_out(&mut self) {
        self.open_export_dialog();
        if let Some(draft) = &mut self.session.file.export_pending {
            draft.layout = true;
            draft.selected_only = false;
        }
    }

    /// The view as drawn, captured for saving as a picture.
    pub(super) fn save_picture(&mut self) {
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.renderer
                .request_capture(render_wgpu::CaptureKind::Scene);
            self.redraw_needed = true;
        }
    }

    pub(super) fn cancel_kernel_job(&mut self) {
        self.kernel_worker.cancel_current();
        app_log::info("Cancelling the running kernel job…");
    }

    pub(super) fn toggle_note_tool(&mut self) {
        self.session.tools.note_tool = !self.session.tools.note_tool;
        self.session.tools.note_card = None;
        if self.session.tools.note_tool {
            app_log::info("Note: click a face, an edge or a point of a part");
        }
    }

    pub(super) fn set_pick_filter(&mut self, filter: crate::ui::PickFilter) {
        self.session.view.pick_filter = filter;
        self.session.hover.edge = None;
        app_log::info(format!("Picking: {}", filter.label()));
    }

    /// The whole body selected, as a double click would.
    pub(super) fn select_whole_body(&mut self, body: core_document::BodyId) {
        self.session.viewport_menu = None;
        self.session.selection.face_highlight = None;
        self.session.selection.last_face_hit = None;
        self.session.selection.body = Some(body.0);
        app_log::info(format!("Selected body: {:?}", body.0));
    }

    /// The camera's field of view, and the setting with it: a later fit
    /// frames with the setting, so it follows the edit at once.
    pub(super) fn set_field_of_view(&mut self, degrees: f32) {
        self.session
            .view
            .camera
            .set_field_of_view(degrees, &self.user_settings.camera);
        self.user_settings.camera.fov_degrees = self.session.view.camera.field_of_view_deg();
    }

    /// Isolation on `body` (or what is selected), or off when it is on.
    pub(super) fn isolate(&mut self, body: Option<core_document::BodyId>) {
        if self.session.document.isolation().is_some() {
            self.session.document.set_isolation(None);
            return;
        }
        let kept = match body {
            Some(body) => vec![body],
            None => self.bodies_to_isolate(),
        };
        if kept.is_empty() {
            app_log::warn("Select a body to isolate");
        } else {
            self.session
                .selection
                .edges
                .retain(|e| kept.iter().any(|b| b.0 == e.body));
            self.session.document.set_isolation(Some(kept));
        }
    }

    /// Every body but `body` (or the selected one) hidden, as one step.
    pub(super) fn hide_others(&mut self, body: Option<core_document::BodyId>) {
        let Some(keep) = body.or_else(|| self.selected_body_for_isolate()) else {
            app_log::warn("Select a body to isolate");
            return;
        };
        let others: Vec<core_document::BodyId> = self
            .session
            .document
            .bodies()
            .iter()
            .map(|b| b.id)
            .filter(|b| *b != keep)
            .collect();
        for other in others {
            self.session.document.set_body_visible(other, false);
        }
        self.session.document.set_body_visible(keep, true);
        self.session.selection.edges.retain(|e| e.body == keep.0);
        self.session.journal.label_next("Isolate body");
        self.close_gesture();
    }

    /// Every hidden body shown and isolation left, as one step.
    pub(super) fn show_all_bodies(&mut self) {
        let hidden: Vec<core_document::BodyId> = self
            .session
            .document
            .bodies()
            .iter()
            .filter(|b| b.hidden)
            .map(|b| b.id)
            .collect();
        for body in hidden {
            self.session.document.set_body_visible(body, true);
        }
        self.session.document.set_isolation(None);
        self.session.journal.label_next("Show all bodies");
        self.close_gesture();
    }

    pub(super) fn set_document_agent_rules(&mut self, rules: String) {
        self.session.document.set_agent_rules(rules);
        self.session.journal.label_next("Agent rules");
        self.close_gesture();
    }

    pub(super) fn set_variable(
        &mut self,
        set: core_document::FeatureId,
        name: String,
        formula: &str,
        comment: Option<&str>,
    ) {
        let label = format!("Set {name}");
        let done = self
            .session
            .document
            .set_variable(set, &name, formula, comment);
        self.finish_variable_edit(done, label);
    }

    pub(super) fn remove_variable(&mut self, set: core_document::FeatureId, name: String) {
        let done = self.session.document.remove_variable(set, &name);
        self.finish_variable_edit(done, format!("Remove {name}"));
    }

    pub(super) fn rename_variable(
        &mut self,
        set: core_document::FeatureId,
        name: String,
        to: &str,
    ) {
        let done = self.session.document.rename_variable(set, &name, to);
        self.finish_variable_edit(done, format!("Rename {name}"));
    }

    /// The orbit pivot put on the point under the cursor.
    pub(super) fn pivot_at_cursor(&mut self) {
        if self.input.cursor_in_viewport.is_some()
            && self
                .session
                .view
                .camera
                .pivot_from_key_h(&self.user_settings.camera)
        {
            self.redraw_needed = true;
        }
    }

    pub(super) fn set_measure_unit(&mut self, unit: Option<core_document::Unit>) {
        if unit != self.session.document.measure_unit() {
            self.session.document.set_measure_unit(unit);
            self.session.journal.label_next("Measure unit");
            self.close_gesture();
        }
    }

    /// The measure tool's picks dropped, the tool kept in hand.
    pub(super) fn clear_measure(&mut self) {
        if let Some(picks) = &mut self.session.tools.measure {
            picks.clear();
        }
        self.session.tools.measure_gap = None;
    }

    /// Ask the kernel's work of each body that takes it, as one step.
    pub(super) fn ask_shape_work(&mut self, bodies: Vec<core_document::BodyId>, work: ShapeWork) {
        self.session.viewport_menu = None;
        let document = &mut self.session.document;
        let asked = bodies
            .into_iter()
            .filter(|body| match work {
                ShapeWork::Convert => document.request_mesh_solid(*body),
                ShapeWork::Refine => document.request_body_refine(*body),
                ShapeWork::Repair => document.request_body_repair(*body),
            })
            .count();
        if asked == 0 {
            return;
        }
        let (label, said) = match work {
            ShapeWork::Convert => (
                "Convert to solid",
                format!("Conversion to a solid asked for {asked} mesh(es)"),
            ),
            ShapeWork::Refine => (
                "Refine shape",
                format!("Refine asked for {asked} converted solid(s)"),
            ),
            ShapeWork::Repair => ("Repair shape", format!("Repair asked for {asked} shape(s)")),
        };
        self.session.journal.label_next(label);
        self.close_gesture();
        app_log::info(format!("{said}; undo history cleared"));
    }
}
