//! Host-side undo/redo: shortcuts land here; selection state is revalidated
//! against the restored document after each history jump.

use crate::log_panel as app_log;
use crate::ui::TreeItemId;
use crate::PrintCadApp;

impl PrintCadApp {
    pub(crate) fn perform_undo(&mut self) {
        if !self.history_jump_allowed() {
            return;
        }
        match self.undo.undo(&mut self.document) {
            Some(label) => {
                app_log::info(format!("Undo: {label}"));
                self.after_history_jump();
            }
            None => app_log::info("Nothing to undo"),
        }
    }

    pub(crate) fn perform_redo(&mut self) {
        if !self.history_jump_allowed() {
            return;
        }
        match self.undo.redo(&mut self.document) {
            Some(label) => {
                app_log::info(format!("Redo: {label}"));
                self.after_history_jump();
            }
            None => app_log::info("Nothing to redo"),
        }
    }

    /// Undo while the kernel worker or a document open is in flight would
    /// let a late response resurrect state from the wrong timeline (e.g. a
    /// redo restoring a pre-tessellation placeholder whose response was
    /// already consumed). Block it; imports finish within moments.
    fn history_jump_allowed(&self) -> bool {
        if self.kernel_worker.in_flight() > 0 || self.document_open_in_flight > 0 {
            app_log::warn("Undo/redo unavailable while an import or open is in progress");
            return false;
        }
        true
    }

    /// Clear selection/editing state that dangles after the document was
    /// swapped by undo/redo.
    fn after_history_jump(&mut self) {
        let doc = &self.document;
        let body_exists = |id: core_document::BodyId| doc.bodies().iter().any(|body| body.id == id);
        let feature_exists =
            |id: core_document::FeatureId| doc.feature_tree().get_node(id).is_some();

        if let Some(id) = self.active_body_id {
            if !body_exists(id) {
                self.active_body_id = None;
            }
        }
        if let Some(id) = self.selected_body {
            if !body_exists(core_document::BodyId(id)) {
                self.selected_body = None;
            }
        }
        if let Some(id) = self.hovered_body {
            if !body_exists(core_document::BodyId(id)) {
                self.hovered_body = None;
            }
        }
        match self.tree_selection {
            Some(TreeItemId::Body(id)) if !body_exists(id) => {
                self.tree_selection = Some(TreeItemId::DocumentRoot);
            }
            Some(TreeItemId::Feature(id)) if !feature_exists(id) => {
                self.tree_selection = Some(TreeItemId::DocumentRoot);
            }
            Some(TreeItemId::ImportedObject(id)) if doc.imported_object(id).is_none() => {
                self.tree_selection = Some(TreeItemId::DocumentRoot);
            }
            _ => {}
        }

        self.face_highlight = None;
        self.last_face_hit = None;
        self.hovered_sketch = None;

        let active_object_dangles = self
            .active_document_object
            .map(|id| !feature_exists(id))
            .unwrap_or(false);

        // Solid geometry lives in derived sidecars that snapshot separately
        // from the features that produce them; after a jump, rebuild every
        // part body so the solids always match the restored feature state.
        wb_part::mark_all_part_features_dirty(&mut self.document);

        // If the feature under edit was undone away, end the workbench's
        // editing session so it doesn't write into a deleted feature.
        if active_object_dangles {
            self.active_document_object = None;
            let wb_id = self.active_workbench.0.clone();
            let params = self.interaction_ctx_params();
            self.with_workbench_ctx(&wb_id, params, |wb, ctx| wb.finish_editing(ctx));
        }
    }
}
