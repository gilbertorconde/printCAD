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

impl PrintCadApp {
    /// Hand every body whose repair was asked for, and whose geometry is
    /// not yet the repaired shape, to the kernel worker. The request is an
    /// op, so this runs the same for a local request, a peer's, and a
    /// document reopened before its repair landed.
    pub(crate) fn drive_shape_repairs(&mut self) {
        for body in self.session.document.bodies_awaiting_repair() {
            if self.session.repairs_in_flight.contains(&body.0) {
                continue;
            }
            let Some(blob) = self.session.document.imported_brep_blob_arc(body) else {
                continue;
            };
            let face_colors = self
                .session
                .document
                .imported_brep_face_colors(body)
                .map(<[_]>::to_vec)
                .unwrap_or_default();
            self.session.repairs_in_flight.insert(body.0);
            app_log::info(format!("Repairing `{}`…", self.body_name(body)));
            self.kernel_worker.request_repair(
                body.0,
                blob,
                face_colors,
                TessellationSettings::default(),
            );
        }
    }

    /// Land a repaired shape on its body: the mended snapshot, its mesh and
    /// the checker's verdict, which clears the body from the queue.
    pub(crate) fn apply_shape_repair(
        &mut self,
        body: core_document::BodyId,
        result: kernel_api::RepairResult,
        elapsed: std::time::Duration,
    ) {
        self.session.repairs_in_flight.remove(&body.0);
        let Some(previous) = self.session.document.imported_geometry(body).cloned() else {
            // The body left the document while the repair ran.
            return;
        };
        let name = self.body_name(body);
        self.session
            .document
            .set_imported_brep_data(body, result.brep_blob, result.face_colors);
        let health = result.health;
        let verdict = if health.is_broken() {
            format!(
                "{} defect(s) remain that the repair does not mend",
                health.broken
            )
        } else {
            "the shape checks clean".to_string()
        };
        let mended = if result.mended.is_empty() {
            "nothing to mend".to_string()
        } else {
            result.mended.join(", ")
        };
        self.session.document.set_imported_geometry(
            body,
            core_document::ImportedGeometry {
                mesh: std::sync::Arc::new(result.mesh),
                bounds_mm: result.bounds_mm.or(previous.bounds_mm),
                health: Some(health),
                ..previous
            },
        );
        if self.session.face_highlight.as_ref().map(|f| f.body) == Some(body.0) {
            // The face sub-mesh belongs to the replaced shape.
            self.session.face_highlight = None;
            self.session.last_face_hit = None;
        }
        self.session.hovered_face = None;
        app_log::info(format!(
            "Repaired `{name}` in {:.0}ms: {mended}; {verdict}",
            elapsed.as_secs_f64() * 1000.0
        ));
    }

    /// A body's name, for log lines.
    pub(crate) fn body_name(&self, body: core_document::BodyId) -> String {
        self.session
            .document
            .bodies()
            .iter()
            .find(|b| b.id == body)
            .map(|b| b.name.clone())
            .unwrap_or_else(|| "body".to_string())
    }
}
