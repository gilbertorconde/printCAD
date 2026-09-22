//! Host-side undo/redo: shortcuts land here; selection state is revalidated
//! against the restored document after each history jump.

use crate::PrintCadApp;
use crate::log_panel as app_log;
use crate::ui::TreeItemId;

impl PrintCadApp {
    pub(crate) fn perform_undo(&mut self) {
        if !self.history_jump_allowed() {
            return;
        }
        match self.session.journal.undo(&mut self.session.document) {
            Some(label) => {
                app_log::info(format!("Undo: {label}"));
                self.after_history_jump();
                // No Rebase: an op-journal undo IS ordinary forward ops —
                // the server log stays truthful and peers hear the undo.
            }
            None => app_log::info("Nothing to undo"),
        }
    }

    pub(crate) fn perform_redo(&mut self) {
        if !self.history_jump_allowed() {
            return;
        }
        match self.session.journal.redo(&mut self.session.document) {
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
        if self.kernel_worker.in_flight() > 0 || self.session.server.status().opens_in_flight > 0 {
            app_log::warn("Undo/redo unavailable while an import or open is in progress");
            return false;
        }
        true
    }

    /// Clear selection/editing state that dangles after the document was
    /// swapped by undo/redo.
    fn after_history_jump(&mut self) {
        let doc = &self.session.document;
        let body_exists = |id: core_document::BodyId| doc.bodies().iter().any(|body| body.id == id);
        let feature_exists =
            |id: core_document::FeatureId| doc.feature_tree().get_node(id).is_some();

        if let Some(id) = self.session.active_body_id
            && !body_exists(id)
        {
            self.session.active_body_id = None;
        }
        if let Some(id) = self.session.selected_body
            && !body_exists(core_document::BodyId(id))
        {
            self.session.selected_body = None;
        }
        if let Some(id) = self.session.hovered_body
            && !body_exists(core_document::BodyId(id))
        {
            self.session.hovered_body = None;
        }
        match self.session.tree_selection {
            Some(TreeItemId::Body(id)) if !body_exists(id) => {
                self.session.tree_selection = Some(TreeItemId::DocumentRoot);
            }
            Some(TreeItemId::Feature(id)) if !feature_exists(id) => {
                self.session.tree_selection = Some(TreeItemId::DocumentRoot);
            }
            Some(TreeItemId::ImportedObject(id)) if doc.imported_object(id).is_none() => {
                self.session.tree_selection = Some(TreeItemId::DocumentRoot);
            }
            _ => {}
        }

        self.session.face_highlight = None;
        self.session.last_face_hit = None;
        self.session.hovered_feature = None;

        let active_object_dangles = self
            .session
            .active_document_object
            .map(|id| !feature_exists(id))
            .unwrap_or(false);

        // Solid geometry lives in derived sidecars that snapshot separately
        // from the features that produce them; after a jump, every bench
        // rebuilds its bodies so the solids match the restored features.
        self.registry.invalidate_all(&mut self.session.document);

        // If the feature under edit was undone away, end the workbench's
        // editing session so it doesn't write into a deleted feature.
        if active_object_dangles {
            self.session.active_document_object = None;
            let wb_id = self.session.active_workbench.0.clone();
            let params = self.interaction_ctx_params();
            if let Some(((), outcome)) =
                self.with_workbench_ctx(&wb_id, params, |wb, ctx| wb.finish_editing(ctx))
            {
                self.apply_hook_outcome(outcome, crate::app::workbench_host::HookSite::Interaction);
            }
        }
    }
}
