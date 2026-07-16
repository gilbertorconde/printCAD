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
                    if let Err(err) =
                        self.apply_step_import(&path, model, raw_bytes, detail, elapsed)
                    {
                        app_log::error(format!(
                            "Failed to apply STEP import {}: {err}",
                            path.display()
                        ));
                    }
                }
                KernelResponse::StepFailed { path, error } => {
                    app_log::error(format!(
                        "STEP import failed `{}`: {}",
                        path.display(),
                        error
                    ));
                }
                KernelResponse::BodyTessellated {
                    body_id,
                    mesh,
                    elapsed,
                } => {
                    let bid = BodyId(body_id);
                    let Some(prev) = self.document.imported_geometry(bid) else {
                        continue;
                    };
                    let source_asset = prev.source_asset;
                    let bounds_mm = prev.bounds_mm;
                    let brep_blob_path = prev.brep_blob_path.clone();
                    let face_colors_path = prev.face_colors_path.clone();
                    self.document.set_imported_geometry(
                        bid,
                        ImportedGeometry {
                            mesh: Arc::new(mesh),
                            source_asset,
                            revision: 0,
                            bounds_mm,
                            brep_blob_path,
                            face_colors_path,
                        },
                    );
                    app_log::info(format!(
                        "Tessellated `{}` in {:.0}ms (worker)",
                        self.document
                            .bodies()
                            .iter()
                            .find(|b| b.id == bid)
                            .map(|b| b.name.as_str())
                            .unwrap_or("body"),
                        elapsed.as_secs_f64() * 1000.0
                    ));
                }
                KernelResponse::BodyTessellateFailed { body_id, error } => {
                    app_log::error(format!("Tessellation failed for body {}: {error}", body_id));
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
                KernelResponse::SolidFailed { body_id, error } => {
                    let name = self
                        .document
                        .bodies()
                        .iter()
                        .find(|b| b.id == BodyId(body_id))
                        .map(|b| b.name.clone())
                        .unwrap_or_else(|| "body".to_string());
                    app_log::error(format!("Rebuild of `{name}` failed: {error}"));
                }
            }
        }
    }

    /// Register the imported bodies + raw asset bytes on the document and
    /// frame the camera around the new geometry. Mirrors the behaviour of
    /// the previous synchronous `import_step_at` but runs entirely on the UI
    /// thread after the heavy CPU work has completed in the worker.
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
        let asset_id = self.document.add_asset_with_data(asset, raw_bytes);

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
        let mut first_body: Option<BodyId> = None;
        let mut body_ids_by_import_index: Vec<BodyId> = Vec::with_capacity(imported_bodies.len());

        for body in imported_bodies {
            let body_id = self.document.create_body(body.name.clone());
            body_ids_by_import_index.push(body_id);
            if first_body.is_none() {
                first_body = Some(body_id);
            }
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

            let has_brep = !body.brep_blob.is_empty();
            self.document
                .set_imported_brep_data(body_id, body.brep_blob, body.face_colors.clone());
            self.document.set_imported_geometry(
                body_id,
                ImportedGeometry {
                    mesh: Arc::new(body.mesh),
                    source_asset: Some(asset_id),
                    revision: 0,
                    bounds_mm: body.bounds_mm,
                    brep_blob_path: None,
                    face_colors_path: None,
                },
            );

            if has_brep {
                if let Some(blob) = self.document.imported_brep_blob_arc(body_id) {
                    self.kernel_worker.request_tessellate_body(
                        body_id.0,
                        blob,
                        body.face_colors,
                        detail.clone(),
                    );
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
        if !nodes_map.is_empty() {
            self.document.append_imported_object_graph(roots, nodes_map);
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

        // On a fresh document, adopt the STEP file's declared unit as the
        // document's display unit. Otherwise leave the user's choice intact —
        // mixing two STEP files with different units shouldn't silently flip
        // the active document's display.
        if was_fresh_document {
            if let Some(unit) = detected_unit {
                self.document.set_display_unit(unit);
                app_log::info(format!(
                    "Display unit set to {} from imported STEP `{}`",
                    unit.short_label(),
                    path.display()
                ));
            }
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

        self.undo.commit(&self.document, "Import STEP");

        Ok(())
    }
}
