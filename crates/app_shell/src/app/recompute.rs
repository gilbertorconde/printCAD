//! Parametric recompute driver for Part Design bodies.
//!
//! Once per frame the app checks for dirty part features, converts each
//! affected body's feature history into a kernel extrude chain, and hands
//! it to the kernel worker. Responses are folded back into the document by
//! `drain_kernel_responses`.

use kernel_api::TessellationSettings;

use crate::log_panel as app_log;
use crate::PrintCadApp;

impl PrintCadApp {
    pub(crate) fn drive_part_recompute(&mut self) {
        let bodies = wb_part::pending_body_rebuilds(&self.document);
        for body_id in bodies {
            // Clear the dirty flags up front: the rebuild is now scheduled
            // (or has failed with a logged error); either way re-submitting
            // every frame would loop.
            for feature_id in wb_part::part_feature_ids(&self.document, body_id) {
                self.document.clear_feature_dirty(feature_id);
            }
            // Sketches only get dirty as rebuild inputs; clear those too.
            let dirty_sketches: Vec<_> = self
                .document
                .feature_tree()
                .all_nodes()
                .filter(|(_, n)| n.workbench_id.as_str() == "wb.sketch" && n.dirty)
                .map(|(id, _)| *id)
                .collect();
            for id in dirty_sketches {
                self.document.clear_feature_dirty(id);
            }

            self.document.clear_body_feature_errors(body_id);
            match wb_part::body_build_ops(&self.document, body_id) {
                Ok(plan) if plan.ops.is_empty() => {
                    self.document.remove_imported_geometry(body_id);
                }
                Ok(plan) => {
                    self.kernel_worker.request_build_solid(
                        body_id.0,
                        plan.ops,
                        plan.op_features.iter().map(|id| id.0).collect(),
                        TessellationSettings::default(),
                    );
                }
                Err(err) => {
                    if let Some(feature) = err.feature {
                        self.document
                            .set_feature_error(feature, Some(err.message.clone()));
                    }
                    app_log::warn(format!("Recompute skipped: {err}"));
                }
            }
        }
    }
}
