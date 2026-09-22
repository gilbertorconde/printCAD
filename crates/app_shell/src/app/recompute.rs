//! Parametric recompute driver.
//!
//! Once per frame the app asks every registered workbench, through the
//! registry, which bodies it wants rebuilt and with what plan, and hands
//! each plan to the kernel worker. Responses are folded back into the
//! document by `drain_kernel_responses`.

use kernel_api::TessellationSettings;

use crate::PrintCadApp;
use crate::log_panel as app_log;

impl PrintCadApp {
    pub(crate) fn drive_part_recompute(&mut self) {
        for job in self.registry.rebuild_jobs(&mut self.session.document) {
            let body_id = job.body;
            self.session.document.clear_body_feature_errors(body_id);
            match job.plan {
                Ok(plan) if plan.ops.is_empty() => {
                    // Only geometry the features produced is cleared; an
                    // imported solid outlives an empty history.
                    if !self.session.document.body_solid_is_imported(body_id) {
                        self.session.document.remove_imported_geometry(body_id);
                    }
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
                        self.session
                            .document
                            .set_feature_error(feature, Some(err.message.clone()));
                    }
                    app_log::warn(format!("Recompute skipped: {err}"));
                }
            }
        }
    }
}
