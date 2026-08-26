//! STEP import: kernel-worker submission and response application.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use core_document::{BodyId, ImportedGeometry, Unit};
use glam::Vec3;
use kernel_api::{ImportedModel, LengthUnit, TessellationSettings};
use tracing::info;
use uuid::Uuid;

use crate::app::frame::aabb_fit_center_radius;
use crate::kernel_worker::KernelResponse;
use crate::log_panel as app_log;
use crate::ui::TreeItemId;
use crate::PrintCadApp;

/// Map a STEP-declared length unit onto the document's display unit enum.
fn length_unit_to_document_unit(unit: LengthUnit) -> Unit {
    match unit {
        LengthUnit::Millimetre => Unit::Mm,
        LengthUnit::Centimetre => Unit::Cm,
        LengthUnit::Metre => Unit::M,
        LengthUnit::Inch => Unit::In,
        LengthUnit::Foot => Unit::Ft,
    }
}

impl PrintCadApp {
    /// Submit a STEP/STP import to the kernel worker. Returns immediately;
    /// the response is delivered later via `drain_kernel_responses` and the
    /// document mutation happens in `apply_step_import` once the worker is
    /// done. Logging the start/finish here keeps the user oriented while the
    /// import is in flight.
    pub(crate) fn import_step_at(&mut self, path: &Path, detail: TessellationSettings) {
        app_log::info(format!("Importing STEP `{}`...", path.display()));
        self.kernel_worker
            .request_step_import(path.to_path_buf(), detail);
    }

    /// Drain any STEP responses that have arrived from the kernel worker and
    /// fold them into the document. Called once per frame in `about_to_wait`.
    /// Whether a kernel error is the user's own cancellation rather than a
    /// geometry failure. The kernel reports it as an ordinary `Err` carrying
    /// `OgeomError::Cancelled`'s message, so match on that.
    fn is_cancellation(error: &str) -> bool {
        error.contains("cancelled")
    }

    pub(crate) fn drain_kernel_responses(&mut self) {
        for response in self.kernel_worker.drain() {
            match response {
                KernelResponse::StepImported {
                    path,
                    model,
                    raw_bytes,
                    detail,
                    elapsed,
                } => {
                    if let Some(route) = self.remote_import_routes.remove(&path) {
                        // A peer's import, re-derived locally: the bodies
                        // already exist (their ImportModel op made them);
                        // only the derived geometry lands here.
                        self.apply_remote_import_geometry(route, model);
                        let _ = std::fs::remove_file(&path);
                    } else if let Err(err) =
                        self.apply_step_import(&path, model, raw_bytes, detail, elapsed)
                    {
                        app_log::error(format!(
                            "Failed to apply STEP import {}: {err}",
                            path.display()
                        ));
                    }
                }
                KernelResponse::StepFailed { path, error } => {
                    if Self::is_cancellation(&error) {
                        app_log::info(format!("STEP import cancelled `{}`", path.display()));
                        continue;
                    }
                    app_log::error(format!(
                        "STEP import failed `{}`: {}",
                        path.display(),
                        error
                    ));
                }
                KernelResponse::SolidBuilt {
                    body_id,
                    result,
                    elapsed,
                } => {
                    let bid = BodyId(body_id);
                    if !self.document.bodies().iter().any(|b| b.id == bid) {
                        // Body deleted (e.g. undo) while the rebuild ran.
                        continue;
                    }
                    let bounds_mm = result.bounds_mm;
                    self.document
                        .set_imported_brep_data(bid, result.brep_blob, Vec::new());
                    self.document.set_imported_geometry(
                        bid,
                        ImportedGeometry {
                            mesh: Arc::new(result.mesh),
                            source_asset: None,
                            revision: 0,
                            bounds_mm,
                            brep_blob_path: None,
                            face_colors_path: None,
                        },
                    );
                    if self.face_highlight.as_ref().map(|f| f.body) == Some(body_id) {
                        // The face sub-mesh belongs to the replaced solid.
                        self.face_highlight = None;
                        self.last_face_hit = None;
                    }
                    app_log::info(format!(
                        "Rebuilt `{}` in {:.0}ms",
                        self.document
                            .bodies()
                            .iter()
                            .find(|b| b.id == bid)
                            .map(|b| b.name.as_str())
                            .unwrap_or("body"),
                        elapsed.as_secs_f64() * 1000.0
                    ));
                }
                KernelResponse::SolidFailed {
                    body_id,
                    failed_feature,
                    error,
                } => {
                    let name = self
                        .document
                        .bodies()
                        .iter()
                        .find(|b| b.id == BodyId(body_id))
                        .map(|b| b.name.clone())
                        .unwrap_or_else(|| "body".to_string());
                    // A cancellation is not a defect: leave the feature
                    // unbadged and the body on its last good solid. The next
                    // edit marks it dirty and rebuilds.
                    if Self::is_cancellation(&error) {
                        app_log::info(format!("Rebuild of `{name}` cancelled"));
                        continue;
                    }
                    // Pin the failure on the culprit feature; the panel and
                    // tree surface it. Downstream keeps the last good solid.
                    if let Some(feature) = failed_feature {
                        self.document.set_feature_error(
                            core_document::FeatureId(feature),
                            Some(error.clone()),
                        );
                    }
                    app_log::error(format!("Rebuild of `{name}` failed: {error}"));
                }
            }
        }
    }

    /// Register the imported bodies + raw asset bytes on the document and
    /// frame the camera around the new geometry. Mirrors the behaviour of
    /// the previous synchronous `import_step_at` but runs entirely on the UI
    /// thread after the heavy CPU work has completed in the worker.
    /// Land a re-derived remote import's meshes on the bodies the peer's
    /// op created. Import order is the correspondence — the kernel is
    /// deterministic at any thread count, so the n-th imported body is the
    /// n-th id the op carried.
    fn apply_remote_import_geometry(
        &mut self,
        route: crate::RemoteImportRoute,
        imported: ImportedModel,
    ) {
        let bodies = imported.bodies;
        if bodies.len() != route.body_ids.len() {
            app_log::error(format!(
                "Remote import mismatch: peer created {} bodies, re-derivation produced {} — geometry left empty",
                route.body_ids.len(),
                bodies.len()
            ));
            return;
        }
        let count = bodies.len();
        for (body, body_id) in bodies.into_iter().zip(&route.body_ids) {
            self.document.set_imported_brep_data(
                *body_id,
                body.brep_blob,
                body.face_colors.clone(),
            );
            self.document.set_imported_geometry(
                *body_id,
                ImportedGeometry {
                    mesh: Arc::new(body.mesh),
                    source_asset: Some(route.asset_id),
                    revision: 0,
                    bounds_mm: body.bounds_mm,
                    brep_blob_path: None,
                    face_colors_path: None,
                },
            );
        }
        app_log::info(format!("Remote import materialized: {count} bodies"));
    }

    fn apply_step_import(
        &mut self,
        path: &Path,
        imported: ImportedModel,
        raw_bytes: Vec<u8>,
        detail: TessellationSettings,
        elapsed: Duration,
    ) -> Result<()> {
        let apply_start = Instant::now();
        // Capture "fresh document" *before* we start mutating it so the
        // auto-unit pick below isn't confused by bodies we're about to add.
        let was_fresh_document = self.document.bodies().is_empty()
            && !self.document.assets().any(|_| true)
            && self.document.imported_geometries().next().is_none();

        let ImportedModel {
            bodies: imported_bodies,
            nodes: imported_nodes,
            source_unit,
        } = imported;

        if imported_bodies.is_empty() {
            app_log::warn(format!(
                "STEP import produced no geometry: {}",
                path.display()
            ));
            return Ok(());
        }
        let detected_unit = source_unit.map(length_unit_to_document_unit);

        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_else(|| "step".to_string());
        let asset = core_document::AssetReference::new(
            format!("assets/{}.{}", uuid::Uuid::new_v4(), extension),
            core_document::AssetType::Step,
            serde_json::json!({
                "source_path": path.display().to_string(),
                "body_count": imported_bodies.len(),
            }),
        );
        let asset_id = asset.id;

        let pending_note = if imported_bodies
            .iter()
            .any(|b| !b.brep_blob.is_empty() && b.mesh.positions.is_empty())
        {
            "background tessellation may still be running"
        } else {
            "mesh from import (no pending tessellation)"
        };

        let mut total_triangles: usize = 0;
        let mut combined_min = [f32::INFINITY; 3];
        let mut combined_max = [f32::NEG_INFINITY; 3];

        // Identities are resolved here, before any document write, so the
        // whole import can land as ONE op with the derived geometry keyed
        // to the same ids afterwards.
        let import_epoch_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let mut unnamed_index = self.document.bodies().len();
        let body_inits: Vec<core_document::op::ImportedBodyInit> = imported_bodies
            .iter()
            .map(|body| {
                let name = body.name.clone().unwrap_or_else(|| {
                    unnamed_index += 1;
                    format!("body{unnamed_index}")
                });
                core_document::op::ImportedBodyInit {
                    id: BodyId::new(),
                    name,
                    created_at: import_epoch_ms,
                }
            })
            .collect();
        let body_ids_by_import_index: Vec<BodyId> = body_inits.iter().map(|init| init.id).collect();
        let first_body = body_ids_by_import_index.first().copied();

        for body in &imported_bodies {
            total_triangles += body.mesh.indices.len() / 3;
            if let Some((min, max)) = body.bounds_mm {
                for axis in 0..3 {
                    combined_min[axis] = combined_min[axis].min(min[axis]);
                    combined_max[axis] = combined_max[axis].max(max[axis]);
                }
            } else if let Some((min, max)) = body.mesh.bounds() {
                for axis in 0..3 {
                    combined_min[axis] = combined_min[axis].min(min[axis]);
                    combined_max[axis] = combined_max[axis].max(max[axis]);
                }
            }
        }

        let mut roots = Vec::new();
        let mut nodes_map = HashMap::new();
        if !imported_nodes.is_empty() {
            let mut id_map = HashMap::with_capacity(imported_nodes.len());
            for src in &imported_nodes {
                id_map.insert(src.id, Uuid::new_v4());
            }
            let mut claimed_body_indices: HashMap<usize, Uuid> = HashMap::new();
            // Preserve C++ FFI preorder DFS emission order so siblings appear
            // in source-file order instead of HashMap-iteration order.
            let mut parent_links_in_order: Vec<(Uuid, Uuid)> =
                Vec::with_capacity(imported_nodes.len());
            for src in imported_nodes {
                let Some(doc_id) = id_map.get(&src.id).copied() else {
                    continue;
                };
                let parent_id = src.parent_id.and_then(|pid| id_map.get(&pid).copied());
                let body_id = src.body_index.and_then(|idx| {
                    if let std::collections::hash_map::Entry::Vacant(e) =
                        claimed_body_indices.entry(idx)
                    {
                        let out = body_ids_by_import_index.get(idx).copied();
                        if out.is_some() {
                            e.insert(doc_id);
                        }
                        out
                    } else {
                        // The same tessellated body can appear on both an
                        // instance node and its referred prototype node.
                        // Bind it once (first owner) so visibility links stay stable.
                        None
                    }
                });
                let name = src.name.unwrap_or_else(|| match src.kind {
                    kernel_api::ImportedNodeKind::Assembly => "Assembly".to_string(),
                    kernel_api::ImportedNodeKind::Part => "Part".to_string(),
                    kernel_api::ImportedNodeKind::Instance => "Instance".to_string(),
                });
                let node = core_document::ImportedObjectNode {
                    id: doc_id,
                    parent_id,
                    children: Vec::new(),
                    kind: src.kind,
                    name,
                    visible: src.visible,
                    body_id,
                    local_transform: src.local_transform,
                };
                if let Some(parent) = parent_id {
                    parent_links_in_order.push((doc_id, parent));
                } else {
                    roots.push(doc_id);
                }
                nodes_map.insert(doc_id, node);
            }
            for (child, parent) in parent_links_in_order {
                if let Some(parent_node) = nodes_map.get_mut(&parent) {
                    parent_node.children.push(child);
                }
            }
        }
        // On a fresh document the file's declared unit becomes the display
        // unit, and it rides inside the import op; on a working document the
        // user's choice stands.
        let adopt_unit = if was_fresh_document {
            detected_unit
        } else {
            None
        };

        // The whole import is one atomic op: asset + bytes + bodies + graph
        // + unit. Geometry lands separately below — derived, not replicated.
        self.document.apply_import(
            asset,
            raw_bytes,
            detail.clone(),
            body_inits,
            roots,
            nodes_map.into_values().collect(),
            adopt_unit,
        );

        for (body, body_id) in imported_bodies.into_iter().zip(&body_ids_by_import_index) {
            self.document.set_imported_brep_data(
                *body_id,
                body.brep_blob,
                body.face_colors.clone(),
            );
            self.document.set_imported_geometry(
                *body_id,
                ImportedGeometry {
                    mesh: Arc::new(body.mesh),
                    source_asset: Some(asset_id),
                    revision: 0,
                    bounds_mm: body.bounds_mm,
                    brep_blob_path: None,
                    face_colors_path: None,
                },
            );
        }

        if combined_min[0] <= combined_max[0] {
            let aabb_min = Vec3::new(combined_min[0], combined_min[1], combined_min[2]);
            let aabb_max = Vec3::new(combined_max[0], combined_max[1], combined_max[2]);
            let (center, radius) = aabb_fit_center_radius(aabb_min, aabb_max);
            self.camera.reset_to_fit(
                center,
                radius,
                Some((aabb_min, aabb_max)),
                &self.user_settings.camera,
            );
        }

        if let Some(body_id) = first_body {
            self.active_body_id = Some(body_id);
            self.tree_selection = Some(TreeItemId::Body(body_id));
            self.selected_body = Some(body_id.0);
        }

        if let Some(unit) = adopt_unit {
            app_log::info(format!(
                "Display unit set to {} from imported STEP `{}`",
                unit.short_label(),
                path.display()
            ));
        }

        crate::app::doc_io::write_recent_dir(path);
        let apply_ms = apply_start.elapsed().as_secs_f64() * 1000.0;
        info!(
            path = %path.display(),
            apply_to_document_ms = format!("{apply_ms:.2}"),
            worker_elapsed_ms = format!("{:.2}", elapsed.as_secs_f64() * 1000.0),
            "STEP import timing (UI thread apply)"
        );
        app_log::info(format!(
            "Imported STEP `{}` in {:.0}ms worker + {:.1}ms apply: {} bodies ({} triangles; {pending_note})",
            path.display(),
            elapsed.as_secs_f64() * 1000.0,
            apply_ms,
            self.document
                .imported_geometries()
                .filter(|(_, g)| g.source_asset == Some(asset_id))
                .count(),
            total_triangles,
        ));

        // ImportModel is a history barrier; closing the boundary clears undo.
        self.journal.note(&mut self.document);

        Ok(())
    }
}
