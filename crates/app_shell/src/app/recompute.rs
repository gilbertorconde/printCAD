//! Parametric recompute driver.
//!
//! Once per frame the app asks every registered workbench, through the
//! registry, which bodies it wants rebuilt and with what plan, and hands
//! each plan to the kernel worker. Responses are folded back into the
//! document by `drain_kernel_responses`.

use core_document::{FeatureId, RebuildJob};
use kernel_api::TessellationSettings;

use crate::PrintCadApp;
use crate::log_panel as app_log;

impl PrintCadApp {
    pub(crate) fn drive_part_recompute(&mut self) {
        self.sync_feature_preview();
        // Nothing is out on the kernel thread: a body still marked as
        // building lost its answer (it left the tab while it built), and
        // what waits for it goes now.
        if self.kernel_worker.in_flight() == 0 && !self.session.builds_in_flight.is_empty() {
            self.session.stale_builds.clear();
            let waiting: Vec<_> = self.session.builds_in_flight.drain().collect();
            for (body, next) in waiting {
                if let Some(next) = next {
                    self.submit_build(body, next);
                }
            }
        }
        self.settle_coarse_builds();
        for job in self.registry.rebuild_jobs(&mut self.session.document) {
            let body_id = job.body;
            self.session.document.clear_body_feature_errors(body_id);
            let job = match job.plan {
                Ok(mut plan) => {
                    if let Some(error) = plan.failed.take() {
                        self.plan_failed(&error, &std::mem::take(&mut plan.unbuilt));
                    }
                    RebuildJob {
                        body: body_id,
                        plan: Ok(plan),
                    }
                }
                plan => RebuildJob {
                    body: body_id,
                    plan,
                },
            };
            match job.plan {
                Ok(plan) if plan.ops.is_empty() => {
                    self.session.coarse.remove(&body_id.0);
                    self.session.preview_rest.remove(&body_id.0);
                    // A build still out is of a history that is gone.
                    if let Some(waiting) = self.session.builds_in_flight.get_mut(&body_id.0) {
                        *waiting = None;
                        self.session.stale_builds.insert(body_id.0);
                        self.drop_build_out(body_id.0);
                    }
                    // Only geometry the features produced is cleared; an
                    // imported solid outlives an empty history.
                    if !self.session.document.body_solid_is_imported(body_id) {
                        self.session.document.remove_imported_geometry(body_id);
                    }
                }
                Ok(plan) => {
                    let preview = self
                        .session
                        .preview_feature
                        .filter(|f| plan.op_features.contains(f))
                        .map(|f| f.0);
                    let mut build = QueuedBuild {
                        ops: plan.ops,
                        op_features: plan.op_features.iter().map(|id| id.0).collect(),
                        preview,
                        probes: plan.probes,
                        partial: false,
                    };
                    // The edited feature first: what follows it waits
                    // until the edits settle.
                    self.session.preview_rest.remove(&body_id.0);
                    if let Some(first) = build.up_to_edited() {
                        self.session
                            .preview_rest
                            .insert(body_id.0, std::mem::replace(&mut build, first));
                    }
                    // One build per body at a time; a newer plan waits in
                    // its place, replacing any older one waiting.
                    // A plan that replaces one still building means the
                    // body is being changed faster than it builds: it is
                    // moving, and builds coarse until it settles.
                    self.session.coarse.remove(&body_id.0);
                    match self.session.builds_in_flight.get_mut(&body_id.0) {
                        Some(waiting) => {
                            *waiting = Some(build);
                            self.session
                                .moving
                                .insert(body_id.0, web_time::Instant::now());
                            self.drop_build_out(body_id.0);
                        }
                        None => self.submit_build(body_id.0, build),
                    }
                }
                Err(err) => self.plan_failed(&err, &[]),
            }
        }
    }

    /// A feature that could not be planned: the error on it, and on each
    /// feature after it a word that it was not built.
    fn plan_failed(&mut self, err: &core_document::BuildError, unbuilt: &[FeatureId]) {
        let name = err
            .feature
            .and_then(|f| self.session.document.get_feature_meta(f))
            .map(|n| format!("`{}`: ", n.name))
            .unwrap_or_default();
        if let Some(feature) = err.feature {
            self.session
                .document
                .set_feature_error(feature, Some(err.message.clone()));
            self.mark_unbuilt(feature, unbuilt.iter().copied());
        }
        app_log::warn(format!("Recompute skipped: {name}{err}"));
    }

    /// Features left out of a build because `failed`, before them, fails.
    pub(crate) fn mark_unbuilt(
        &mut self,
        failed: FeatureId,
        unbuilt: impl IntoIterator<Item = FeatureId>,
    ) {
        mark_unbuilt(&mut self.session.document, failed, unbuilt);
    }
}

/// Mark the features left out of a build because `failed`, before them,
/// fails.
pub(crate) fn mark_unbuilt(
    document: &mut core_document::Document,
    failed: FeatureId,
    unbuilt: impl IntoIterator<Item = FeatureId>,
) {
    let name = document
        .get_feature_meta(failed)
        .map(|n| n.name.clone())
        .unwrap_or_default();
    for feature in unbuilt {
        if feature != failed {
            document.set_feature_error(
                feature,
                Some(format!("not built: `{name}` before it fails")),
            );
        }
    }
}

/// How long a body's plans must stop changing before a coarse solid is
/// built again at full detail.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(300);

/// A body's build plan, as it goes to the kernel thread.
#[derive(Clone)]
pub(crate) struct QueuedBuild {
    ops: Vec<kernel_api::SolidOp>,
    op_features: Vec<uuid::Uuid>,
    preview: Option<uuid::Uuid>,
    probes: Vec<core_document::PlanProbe>,
    /// It stops at the edited feature.
    partial: bool,
}

impl QueuedBuild {
    /// The build of the history up to the end of the edited feature, when
    /// features follow it: what the open task shows, built first.
    fn up_to_edited(&self) -> Option<QueuedBuild> {
        let feature = self.preview?;
        let end = self.op_features.iter().rposition(|f| *f == feature)? + 1;
        (end < self.ops.len()).then(|| QueuedBuild {
            ops: self.ops[..end].to_vec(),
            op_features: self.op_features[..end].to_vec(),
            preview: self.preview,
            probes: self
                .probes
                .iter()
                .filter(|p| p.probe.after_op <= end)
                .copied()
                .collect(),
            partial: true,
        })
    }
}

impl PrintCadApp {
    /// How finely built, repaired, converted and replaced solids are meshed
    /// for the scene (Preferences › Display › Rendering, Curve smoothness).
    pub(crate) fn solid_detail(&self) -> TessellationSettings {
        TessellationSettings {
            angular_tolerance_deg: self.user_settings.rendering.curve_step_deg.clamp(2.0, 45.0),
            ..TessellationSettings::default()
        }
    }

    fn submit_build(&mut self, body: uuid::Uuid, build: QueuedBuild) {
        self.session.builds_in_flight.insert(body, None);
        // A moving body is meshed coarse, and its plan kept to build again
        // finely once it settles.
        let moving = self
            .session
            .moving
            .get(&body)
            .is_some_and(|at| at.elapsed() < SETTLE);
        let detail = if moving {
            self.session.coarse.insert(body, build.clone());
            coarse(self.solid_detail())
        } else {
            self.session.coarse.remove(&body);
            self.session.moving.remove(&body);
            self.solid_detail()
        };
        if build.partial {
            self.session.partial_out.insert(body);
        } else {
            self.session.partial_out.remove(&body);
        }
        let serial = self.kernel_worker.request_build_solid(
            body,
            build.ops,
            build.op_features,
            detail,
            build.preview,
            build.probes,
        );
        self.session.build_serials.insert(body, serial);
    }

    /// Build at full detail every body shown coarse whose plans have
    /// stopped changing, and the whole history of every body whose edited
    /// feature was built alone.
    fn settle_coarse_builds(&mut self) {
        let settled: Vec<uuid::Uuid> = self
            .session
            .coarse
            .keys()
            .chain(self.session.preview_rest.keys())
            .filter(|body| !self.session.builds_in_flight.contains_key(*body))
            .filter(|body| {
                self.session
                    .moving
                    .get(*body)
                    .is_none_or(|at| at.elapsed() >= SETTLE)
            })
            .copied()
            .collect();
        for body in settled {
            // The whole history, when the edited feature was built alone;
            // it is built at full detail and brings the preview with it.
            let coarse = self.session.coarse.remove(&body);
            if let Some(build) = self.session.preview_rest.remove(&body).or(coarse) {
                self.session.moving.remove(&body);
                self.submit_build(body, build);
            }
        }
    }

    /// The body's build out on the kernel thread is of a plan nobody needs
    /// any more: drop it, so the plan waiting goes sooner.
    fn drop_build_out(&mut self, body: uuid::Uuid) {
        if self.session.dropped_builds.contains(&body) {
            return;
        }
        let Some(&serial) = self.session.build_serials.get(&body) else {
            return;
        };
        let usual = self.session.build_times.get(&body).copied();
        if self.kernel_worker.drop_build(serial, usual) {
            self.session.dropped_builds.insert(body);
        }
    }

    /// A body's build landed: the newest plan made meanwhile goes now.
    pub(crate) fn build_landed(&mut self, body: uuid::Uuid) {
        self.session.build_serials.remove(&body);
        if let Some(Some(next)) = self.session.builds_in_flight.remove(&body) {
            self.submit_build(body, next);
        }
    }

    /// Follow the feature the active bench edits: while a task edits one
    /// that builds solid, its body is built with a preview of it, and when
    /// the task closes the whole solids go back.
    fn sync_feature_preview(&mut self) {
        let document = &self.session.document;
        let editing = self
            .registry
            .workbench(&self.session.active_workbench.0)
            .ok()
            .and_then(|wb| wb.editing_feature())
            .filter(|feature| {
                document.get_feature_meta(*feature).is_some_and(|node| {
                    node.body.is_some()
                        && self
                            .registry
                            .feature_info(node)
                            .is_some_and(|info| info.builds_solid)
                })
            });
        if editing == self.session.preview_feature {
            return;
        }
        self.end_feature_previews();
        self.session.preview_feature = editing;
        if let Some(feature) = editing {
            // Built again, this time with its preview.
            self.session.document.mark_feature_stale(feature);
        }
    }

    /// Put every body showing a preview back to its whole solid. When the
    /// previewed feature is gone (its task was cancelled), that solid is
    /// not the body's any more: the body is built again from its history.
    pub(crate) fn end_feature_previews(&mut self) {
        let gone = self
            .session
            .preview_feature
            .is_some_and(|f| self.session.document.get_feature_meta(f).is_none());
        for (body, preview) in std::mem::take(&mut self.session.previews) {
            if !self.session.document.bodies().iter().any(|b| b.id == body) {
                continue;
            }
            if gone || !preview.complete {
                self.registry
                    .invalidate_body(&mut self.session.document, body);
            } else {
                store_built_solid(&mut self.session.document, body, preview.full);
            }
        }
        // A body shown at the feature, its history after it still to build.
        for body in std::mem::take(&mut self.session.preview_rest).into_keys() {
            let body = core_document::BodyId(body);
            if self.session.document.bodies().iter().any(|b| b.id == body) {
                self.registry
                    .invalidate_body(&mut self.session.document, body);
            }
        }
    }

    /// A body's build failed while it showed a preview: the tool drawn is
    /// of an earlier build, so it goes, and the body keeps showing itself
    /// without the feature.
    pub(crate) fn drop_failed_preview(&mut self, body: core_document::BodyId) {
        self.session.previews.remove(&body);
    }

    /// A build that came with the edited feature's preview: the body stands
    /// without the feature (before one that adds, after one that cuts) and
    /// the feature's tool is drawn over it, the whole solid kept aside.
    pub(crate) fn show_feature_preview(
        &mut self,
        body: core_document::BodyId,
        full: kernel_api::SolidBuildResult,
        preview: kernel_api::FeaturePreview,
        complete: bool,
    ) {
        let document = &mut self.session.document;
        match preview.shown {
            Some(shown) => store_built_solid(document, body, *shown),
            None => document.remove_imported_geometry(body),
        }
        let placement = document.body_placement(body);
        let tool = if placement.is_identity() {
            preview.tool
        } else {
            placement.mesh(&preview.tool)
        };
        let (id, revision) = match self.session.previews.get(&body) {
            Some(previous) => (previous.id, previous.revision.wrapping_add(1)),
            None => (uuid::Uuid::new_v4(), 0),
        };
        self.session.previews.insert(
            body,
            crate::app::session::BodyPreview {
                full,
                complete,
                tool: std::sync::Arc::new(tool),
                id,
                revision,
            },
        );
    }

    /// Hand every body whose repair was asked for, and whose geometry is
    /// not yet the repaired shape, to the kernel worker. The request is an
    /// op, so this runs the same for a local request, a peer's, and a
    /// document reopened before its repair landed.
    pub(crate) fn drive_shape_repairs(&mut self) {
        for body in self.session.document.bodies_awaiting_repair() {
            if self.session.repairs_in_flight.contains(&body.0) {
                continue;
            }
            // A body with a base mends its base, which its features build on.
            let document = &self.session.document;
            let (blob, face_colors) = if document.has_base_solid(body) {
                (
                    document
                        .base_brep_blob(body)
                        .map(|b| std::sync::Arc::new(b.to_vec())),
                    document.base_face_colors(body).map(<[_]>::to_vec),
                )
            } else {
                (
                    document.imported_brep_blob_arc(body),
                    document.imported_brep_face_colors(body).map(<[_]>::to_vec),
                )
            };
            let Some(blob) = blob else {
                continue;
            };
            let face_colors = face_colors.unwrap_or_default();
            self.session.repairs_in_flight.insert(body.0);
            app_log::info(format!("Repairing `{}`…", self.body_name(body)));
            self.kernel_worker
                .request_repair(body.0, blob, face_colors, self.solid_detail());
        }
    }

    /// Read the new shape of every body whose shape was replaced by
    /// another file (`ReplaceBodyShape`), each once, on the kernel thread.
    pub(crate) fn drive_shape_replacements(&mut self) {
        for (body, asset) in self.session.document.bodies_awaiting_shape() {
            if self.session.shapes_in_flight.contains(&body.0)
                || self.session.shapes_failed.get(&body.0) == Some(&asset)
            {
                continue;
            }
            let Some((reference, bytes)) = self.session.document.asset_with_bytes(asset) else {
                continue;
            };
            // The kernel reads files: a copy of the asset under its format's
            // extension, removed by the worker when read.
            let name = std::path::Path::new(&reference.path)
                .file_name()
                .map(|n| n.to_owned())
                .unwrap_or_else(|| format!("{asset}.step").into());
            let path =
                match crate::platform::scratch_file("shapes", &name.to_string_lossy(), &bytes) {
                    Ok(path) => path,
                    Err(e) => {
                        self.session.shapes_failed.insert(body.0, asset);
                        app_log::error(format!(
                            "Could not stage the new shape of `{}`: {e}",
                            self.body_name(body)
                        ));
                        continue;
                    }
                };
            self.session.shapes_in_flight.insert(body.0);
            app_log::info(format!(
                "Reading the new shape of `{}`…",
                self.body_name(body)
            ));
            self.kernel_worker
                .request_read_solid(body.0, asset, path, self.solid_detail());
        }
    }

    /// Land a body's new shape: as its base, which its features then build
    /// on, or as its shape when it has no history.
    pub(crate) fn apply_shape_read(
        &mut self,
        body: core_document::BodyId,
        asset: uuid::Uuid,
        result: Result<kernel_api::MeshSolidResult, String>,
        elapsed: std::time::Duration,
    ) {
        self.session.shapes_in_flight.remove(&body.0);
        let name = self.body_name(body);
        let read = match result {
            Ok(read) => read,
            Err(error) => {
                self.session.shapes_failed.insert(body.0, asset);
                app_log::error(format!(
                    "The new shape of `{name}` could not be read: {error}"
                ));
                return;
            }
        };
        let geometry = core_document::ImportedGeometry {
            bounds_mm: read.bounds_mm.or_else(|| read.mesh.bounds()),
            mesh: std::sync::Arc::new(read.mesh),
            source_asset: Some(asset),
            revision: 0,
            brep_blob_path: None,
            mesh_path: None,
            face_colors_path: None,
            health: Some(read.health),
        };
        let document = &mut self.session.document;
        if document.has_base_solid(body) {
            document.set_base_solid(body, geometry, read.brep_blob, read.face_colors);
            // Its features build again on the new base.
            self.registry.invalidate_body(document, body);
        } else {
            document.set_imported_geometry(body, geometry);
            document.set_imported_brep_data(body, read.brep_blob, read.face_colors);
        }
        let notes = if read.summary.is_empty() {
            String::new()
        } else {
            format!(" ({})", read.summary.join(", "))
        };
        app_log::success(format!(
            "`{name}` takes its new shape, read in {:.1} s{notes}",
            elapsed.as_secs_f32()
        ));
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
        if let Some(previous) = self.session.document.base_geometry(body).cloned() {
            let broken = result.health.broken;
            self.session.document.set_base_solid(
                body,
                core_document::ImportedGeometry {
                    mesh: std::sync::Arc::new(result.mesh),
                    bounds_mm: result.bounds_mm.or(previous.bounds_mm),
                    health: Some(result.health),
                    ..previous
                },
                result.brep_blob,
                result.face_colors,
            );
            // The features build again on the mended base.
            self.registry
                .invalidate_body(&mut self.session.document, body);
            app_log::info(format!(
                "Repaired the base of `{}` in {:.1} s{}",
                self.body_name(body),
                elapsed.as_secs_f32(),
                if broken > 0 {
                    format!(": {broken} defect(s) remain that the repair does not mend")
                } else {
                    String::new()
                }
            ));
            return;
        }
        let Some(previous) = self.session.document.imported_geometry(body).cloned() else {
            // The body left the document while the repair ran.
            return;
        };
        let name = self.body_name(body);
        self.session
            .document
            .set_imported_brep_data(body, result.brep_blob, result.face_colors);
        // A repair mends the facets; whether they are refined is its own.
        let health = kernel_api::ShapeHealth {
            faceted: previous.health.as_ref().is_some_and(|h| h.faceted),
            ..result.health
        };
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

    /// Hand every converted solid whose refine was asked for, and which
    /// is still in facets, to the kernel worker. The request is an op: a
    /// peer's and a reopened document's refine the same way.
    pub(crate) fn drive_shape_refinements(&mut self) {
        for body in self.session.document.bodies_awaiting_refine() {
            if self.session.refines_in_flight.contains(&body.0)
                || self.session.refines_failed.contains(&body.0)
            {
                continue;
            }
            let document = &self.session.document;
            let Some(blob) = document.imported_brep_blob_arc(body) else {
                continue;
            };
            let face_colors = document
                .imported_brep_face_colors(body)
                .map(<[_]>::to_vec)
                .unwrap_or_default();
            self.session.refines_in_flight.insert(body.0);
            app_log::info(format!("Refining `{}`…", self.body_name(body)));
            self.kernel_worker
                .request_refine(body.0, blob, face_colors, self.solid_detail());
        }
    }

    /// Land a refined solid: its snapshot and mesh in place of the facets.
    /// A refine that failed leaves the facets as they are, said in the log.
    pub(crate) fn apply_shape_refine(
        &mut self,
        body: core_document::BodyId,
        result: Result<kernel_api::MeshSolidResult, String>,
        elapsed: std::time::Duration,
    ) {
        self.session.refines_in_flight.remove(&body.0);
        let name = self.body_name(body);
        let refined = match result {
            Ok(refined) => refined,
            Err(error) => {
                self.session.refines_failed.insert(body.0);
                if Self::is_cancellation(&error) {
                    app_log::info(format!("Refine of `{name}` cancelled"));
                } else {
                    app_log::error(format!(
                        "`{name}` could not be refined and keeps its facets: {error}"
                    ));
                }
                return;
            }
        };
        let Some(previous) = self.session.document.imported_geometry(body).cloned() else {
            // The body left the document while the refine ran.
            return;
        };
        self.session
            .document
            .set_imported_brep_data(body, refined.brep_blob, refined.face_colors);
        self.session.document.set_imported_geometry(
            body,
            core_document::ImportedGeometry {
                mesh: std::sync::Arc::new(refined.mesh),
                bounds_mm: refined.bounds_mm.or(previous.bounds_mm),
                health: Some(refined.health),
                ..previous
            },
        );
        if self.session.face_highlight.as_ref().map(|f| f.body) == Some(body.0) {
            self.session.face_highlight = None;
            self.session.last_face_hit = None;
        }
        self.session.hovered_face = None;
        app_log::info(format!(
            "Refined `{name}` in {:.0}ms: {}",
            elapsed.as_secs_f64() * 1000.0,
            refined.summary.join(", ")
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

impl PrintCadApp {
    /// The body the property panel is showing: a body row, or an imported
    /// part linked to one.
    fn panel_body(&self) -> Option<core_document::BodyId> {
        match self.session.tree_selection? {
            crate::ui::TreeItemId::Body(body) => Some(body),
            crate::ui::TreeItemId::ImportedObject(id) => {
                self.session.document.body_of_imported_object(id)
            }
            _ => None,
        }
    }

    /// Ask the kernel worker to measure the body the property panel shows,
    /// once per revision of its geometry.
    pub(crate) fn drive_measurement(&mut self) {
        let Some(body) = self.panel_body() else {
            return;
        };
        let Some(revision) = self
            .session
            .document
            .imported_geometry(body)
            .map(|g| g.revision)
        else {
            return;
        };
        if self
            .session
            .physical
            .get(&body.0)
            .is_some_and(|(measured, _)| *measured == revision)
        {
            return;
        }
        let Some(blob) = self.session.document.imported_brep_blob_arc(body) else {
            return;
        };
        self.session
            .physical
            .insert(body.0, (revision, crate::ui::Physical::Measuring));
        self.kernel_worker.request_measure(body.0, revision, blob);
    }

    /// The panel body's measure, when it is of the geometry on screen.
    pub(crate) fn panel_physical(&self) -> Option<crate::ui::Physical> {
        let body = self.panel_body()?;
        let revision = self.session.document.imported_geometry(body)?.revision;
        let (measured, reading) = self.session.physical.get(&body.0)?;
        if *measured != revision {
            return None;
        }
        // The kernel measures the body's own shape; the centre is shown
        // where the body sits.
        Some(match reading {
            crate::ui::Physical::Ready(props) => {
                let mut props = *props;
                let placement = self.session.document.body_placement(body);
                let c = props.centre_mm.map(|v| v as f32);
                props.centre_mm = placement.point(c).map(f64::from);
                crate::ui::Physical::Ready(props)
            }
            other => other.clone(),
        })
    }
}

impl PrintCadApp {
    /// Hand every mesh body whose conversion was asked for, and has not
    /// landed, to the kernel worker. The request is an op: a peer's and a
    /// reopened document's convert the same way.
    pub(crate) fn drive_mesh_solids(&mut self) {
        for body in self.session.document.bodies_awaiting_solid() {
            if self.session.solids_in_flight.contains(&body.0) {
                continue;
            }
            // The solid is built in the body's own frame, as every kernel
            // shape is; the scene's copy is placed.
            let Some((mesh, _)) = self.session.document.local_geometry(body) else {
                continue;
            };
            self.session.solids_in_flight.insert(body.0);
            app_log::info(format!(
                "Converting `{}` to a solid ({} triangles)…",
                self.body_name(body),
                mesh.indices.len() / 3
            ));
            self.kernel_worker
                .request_mesh_solid(body.0, mesh, self.solid_detail());
        }
    }

    /// Ask the kernel for the snapshot of every mirrored copy that has
    /// none: its source's, mirrored.
    pub(crate) fn drive_mirrored_copies(&mut self) {
        for (body, blob, plane) in self.session.document.copies_awaiting_shape() {
            let failed = self
                .session
                .mirrors_failed
                .get(&body.0)
                .is_some_and(|f| std::sync::Arc::ptr_eq(f, &blob));
            if !failed && self.session.mirrors_in_flight.insert(body.0) {
                self.kernel_worker.request_mirror(body.0, blob, plane);
            }
        }
    }

    /// Land a mesh body's solid: its snapshot, its mesh with kernel faces
    /// and edges, the checker's verdict, and a line on what it became.
    pub(crate) fn apply_mesh_solid(
        &mut self,
        body: core_document::BodyId,
        result: kernel_api::MeshSolidResult,
        elapsed: std::time::Duration,
    ) {
        self.session.solids_in_flight.remove(&body.0);
        let Some(previous) = self.session.document.imported_geometry(body).cloned() else {
            return;
        };
        let name = self.body_name(body);
        let faceted = result.health.faceted;
        self.session
            .document
            .set_imported_brep_data(body, result.brep_blob, result.face_colors);
        self.session.document.set_imported_geometry(
            body,
            core_document::ImportedGeometry {
                mesh: std::sync::Arc::new(result.mesh),
                bounds_mm: result.bounds_mm.or(previous.bounds_mm),
                health: Some(result.health),
                ..previous
            },
        );
        if self.session.face_highlight.as_ref().map(|f| f.body) == Some(body.0) {
            self.session.face_highlight = None;
            self.session.last_face_hit = None;
        }
        self.session.hovered_face = None;
        let summary = result.summary.join(", ");
        let ms = elapsed.as_secs_f64() * 1000.0;
        if result.closed {
            app_log::info(format!(
                "Converted `{name}` to a solid in {ms:.0}ms: {summary}"
            ));
            if faceted {
                app_log::warn(format!(
                    "`{name}` keeps its curved areas as flat facets: features on or along \
                     them (pads and pockets on a face, fillets) may fail until you refine it \
                     (right-click › Refine shape)"
                ));
            }
        } else {
            app_log::warn(format!(
                "`{name}` does not close, so it became an open shell, not a solid \
                 ({ms:.0}ms): {summary}"
            ));
        }
    }
}

/// Mesh settings for a body being changed faster than it builds: about
/// four times coarser along curves, at most 45° a step round them.
fn coarse(fine: TessellationSettings) -> TessellationSettings {
    TessellationSettings {
        mesh_deviation: fine.mesh_deviation * 4.0,
        chord_tolerance: fine.chord_tolerance * 4.0,
        angular_tolerance_deg: (fine.angular_tolerance_deg * 3.0).min(45.0),
        ..fine
    }
}

/// Put a body's rebuilt solid in the document: its shape and the mesh drawn
/// from it.
pub(crate) fn store_built_solid(
    document: &mut core_document::Document,
    body: core_document::BodyId,
    result: kernel_api::SolidBuildResult,
) {
    let bounds_mm = result.bounds_mm;
    document.set_imported_brep_data(body, result.brep_blob, Vec::new());
    document.set_imported_geometry(
        body,
        core_document::ImportedGeometry {
            mesh: std::sync::Arc::new(result.mesh),
            source_asset: None,
            revision: 0,
            bounds_mm,
            brep_blob_path: None,
            mesh_path: None,
            face_colors_path: None,
            health: None,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A moving body's mesh is coarser along and round its curves, and
    /// keeps everything else of the fine settings.
    #[test]
    fn coarse_settings_are_coarser_and_otherwise_the_same() {
        let fine = TessellationSettings {
            angular_tolerance_deg: 10.0,
            ..TessellationSettings::default()
        };
        let rough = coarse(fine.clone());
        assert!(rough.angular_tolerance_deg > fine.angular_tolerance_deg);
        assert!(rough.mesh_deviation > fine.mesh_deviation);
        assert!(rough.chord_tolerance > fine.chord_tolerance);
        assert_eq!(rough.weld_cross_face, fine.weld_cross_face);
        assert!(coarse(TessellationSettings::default()).angular_tolerance_deg <= 45.0);
    }

    /// The edited feature's build stops at its last op; with nothing after
    /// it, or no feature edited, there is no shorter build.
    #[test]
    fn the_edited_feature_is_built_up_to_its_end() {
        use kernel_api::{BooleanOp, Placement, PrimitiveKind, SolidOp};
        let op = |op| SolidOp::Primitive {
            kind: PrimitiveKind::Box {
                length: 1.0,
                width: 1.0,
                height: 1.0,
            },
            placement: Placement::default(),
            op,
        };
        let [a, b, c] = [
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
        ];
        let build = |preview| QueuedBuild {
            ops: vec![
                op(BooleanOp::NewSolid),
                op(BooleanOp::Fuse),
                op(BooleanOp::Fuse),
                op(BooleanOp::Cut),
            ],
            op_features: vec![a, b, b, c],
            preview,
            probes: Vec::new(),
            partial: false,
        };
        let first = build(Some(b)).up_to_edited().expect("a feature follows");
        assert_eq!(first.ops.len(), 3, "up to the edited feature's last op");
        assert_eq!(first.op_features, vec![a, b, b]);
        assert!(first.partial);
        assert!(build(Some(c)).up_to_edited().is_none(), "the last feature");
        assert!(build(None).up_to_edited().is_none(), "nothing edited");
    }
}
