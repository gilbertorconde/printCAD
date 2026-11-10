//! Base solids, mesh starts and the shape requests (repair, replace,
//! stand-in, convert, refine) the host answers.

use super::*;

/// An asset for a shape read from the file `name` for `body`, its role
/// saying why it was kept.
fn shape_asset(name: &str, role: &str, body: BodyId) -> AssetReference {
    let extension = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "step".to_string());
    AssetReference::new(
        format!("assets/{}.{extension}", Uuid::new_v4()),
        AssetType::from_extension(&extension),
        serde_json::json!({ "source_path": name, role: body.0 }),
    )
}

impl Document {
    /// Keep the body's solid as it stands (from an import, a mesh
    /// conversion or a repair) as the base its feature history starts
    /// from; `on` false puts the base back as the body's shape and drops
    /// it. Whether anything changed: a body with no solid of its own has
    /// nothing to keep, and one already based has nothing to add.
    pub fn set_body_base(&mut self, body: BodyId, on: bool) -> bool {
        let has = self.base_solids.contains_key(&body);
        let possible = if on {
            !has && self.imported_brep_blobs.contains_key(&body) && self.copy_source(body).is_none()
        } else {
            has
        };
        if possible {
            self.record_and_apply(op::DocumentOp::SetBodyBase { id: body, on });
        }
        possible
    }

    /// Keep a mesh body's mesh as it stands as the start its mesh history
    /// builds from (`on`), or put the start back as its mesh. Whether
    /// anything changed: only a mesh body has a mesh start.
    pub fn set_mesh_start(&mut self, body: BodyId, on: bool) -> bool {
        let has = self.mesh_starts.contains_key(&body);
        let possible = if on {
            !has && self.is_mesh_body(body) && self.copy_source(body).is_none()
        } else {
            has
        };
        if possible {
            self.record_and_apply(op::DocumentOp::SetMeshStart { id: body, on });
        }
        possible
    }

    /// The bounded solid a feature offers a mesh boolean, in the
    /// document's frame.
    pub fn feature_solid(&self, id: FeatureId) -> Option<&kernel_api::MeshOperand> {
        self.feature_solids.get(&id)
    }

    /// The mesh a mesh body's history builds from, in its own frame.
    pub fn mesh_start(&self, body: BodyId) -> Option<&ImportedGeometry> {
        self.mesh_starts.get(&body)
    }

    /// Measurements only while the complete history prefix still agrees.
    pub fn mesh_steps(&self, plan: &MeshPlan) -> Option<&[kernel_api::MeshStep]> {
        let built = self.mesh_results.get(&plan.body)?;
        (&built.plan == plan).then_some(&built.steps)
    }

    /// Start vertices behind the displayed mesh of this exact plan.
    pub fn mesh_sources(&self, plan: &MeshPlan) -> Option<&[Option<u32>]> {
        let built = self.mesh_results.get(&plan.body)?;
        (&built.plan == plan).then_some(&built.sources)
    }

    /// Land a mesh history's derived shape, measurements and feature errors.
    pub fn store_mesh_build(&mut self, plan: MeshPlan, answer: kernel_api::MeshChainAnswer) {
        let Some(start) = self.mesh_start(plan.body).cloned() else {
            return;
        };
        for (k, id) in plan.features.iter().enumerate() {
            let error = answer.failed.as_ref().and_then(|(failed, why)| {
                if k == *failed {
                    Some(why.clone())
                } else if k > *failed {
                    Some("not built: an earlier mesh step failed".into())
                } else {
                    None
                }
            });
            self.set_feature_error(*id, error);
        }
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for p in &answer.mesh.positions {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let bounds_mm = (!answer.mesh.positions.is_empty()).then_some((lo, hi));
        let body = plan.body;
        self.mesh_results.insert(
            body,
            rebuild::MeshResult {
                plan,
                steps: answer.steps,
                sources: answer.sources,
            },
        );
        self.set_imported_geometry(
            body,
            ImportedGeometry {
                mesh: std::sync::Arc::new(answer.mesh),
                bounds_mm,
                mesh_path: None,
                ..start
            },
        );
    }

    pub(crate) fn apply_set_mesh_start(&mut self, body: BodyId, on: bool) {
        self.mesh_results.remove(&body);
        if on {
            let Some(mut kept) = self.imported_meshes.get(&body).cloned() else {
                return;
            };
            if let Some((mesh, bounds)) = self.local_geometry(body) {
                kept.mesh = mesh;
                kept.bounds_mm = bounds;
            }
            self.mesh_starts.insert(body, kept);
        } else if let Some(geometry) = self.mesh_starts.remove(&body) {
            self.set_imported_geometry(body, geometry);
        }
        self.mark_dirty();
    }

    /// What the checker found in the shape a body takes from outside its
    /// features: its base solid, else its imported solid.
    pub fn body_health(&self, body: BodyId) -> Option<&kernel_api::ShapeHealth> {
        self.base_solids
            .get(&body)
            .or_else(|| self.imported_geometry(body))?
            .health
            .as_ref()
    }

    /// Replace the body's base solid with what a repair (or another
    /// derivation of the same op) made of it: derived state, no op. The
    /// geometry is in the body's own frame.
    pub fn set_base_solid(
        &mut self,
        body: BodyId,
        geometry: ImportedGeometry,
        brep_blob: Vec<u8>,
        face_colors: Vec<[f32; 3]>,
    ) {
        if !self.base_solids.contains_key(&body) {
            return;
        }
        self.base_solids.insert(body, geometry);
        self.base_brep_blobs
            .insert(body, std::sync::Arc::new(brep_blob));
        self.base_face_colors.insert(body, face_colors);
        self.mark_dirty();
    }

    /// Whether the body's history starts from a base solid.
    pub fn has_base_solid(&self, body: BodyId) -> bool {
        self.base_solids.contains_key(&body)
    }

    /// The snapshot of the body's base solid, in its own frame.
    pub fn base_brep_blob(&self, body: BodyId) -> Option<&[u8]> {
        self.base_brep_blobs.get(&body).map(|b| b.as_slice())
    }

    /// The base solid's face colours, in its snapshot's face order.
    pub fn base_face_colors(&self, body: BodyId) -> Option<&[[f32; 3]]> {
        self.base_face_colors.get(&body).map(Vec::as_slice)
    }

    /// The base solid's geometry as it was kept: the checker's findings,
    /// the asset it came from.
    pub fn base_geometry(&self, body: BodyId) -> Option<&ImportedGeometry> {
        self.base_solids.get(&body)
    }

    pub(crate) fn apply_set_body_base(&mut self, body: BodyId, on: bool) {
        if on {
            let (Some(geometry), Some(blob)) = (
                self.imported_meshes.get(&body).cloned(),
                self.imported_brep_blobs.get(&body).cloned(),
            ) else {
                return;
            };
            let colors = self
                .imported_brep_face_colors
                .get(&body)
                .cloned()
                .unwrap_or_default();
            // The base keeps the body's own frame: its mesh as the body
            // sees it, not as it is placed.
            let mut kept = geometry;
            if let Some((mesh, bounds)) = self.local_geometry(body) {
                kept.mesh = mesh;
                kept.bounds_mm = bounds;
            }
            kept.brep_blob_path = None;
            kept.face_colors_path = None;
            self.base_solids.insert(body, kept);
            self.base_brep_blobs.insert(body, blob);
            self.base_face_colors.insert(body, colors);
        } else {
            let Some(geometry) = self.base_solids.remove(&body) else {
                return;
            };
            let blob = self.base_brep_blobs.remove(&body);
            let colors = self.base_face_colors.remove(&body).unwrap_or_default();
            // The base is the body's shape again, as it was kept.
            self.set_imported_geometry(body, geometry);
            if let Some(blob) = blob {
                self.imported_brep_blobs.insert(body, blob);
                self.imported_brep_face_colors.insert(body, colors);
            }
        }
        self.mark_dirty();
    }

    /// Whether "Repair shape" applies: an imported or base solid the
    /// kernel's checker calls broken, its repair not asked for yet.
    pub fn can_repair(&self, body: BodyId) -> bool {
        self.body_health(body).is_some_and(|h| h.is_broken())
            && self.body(body).is_some_and(|b| !b.repair_requested)
            && (self.body_solid_is_imported(body) || self.has_base_solid(body))
    }

    /// Ask for the kernel's repair on an imported body's shape. Only a body
    /// whose shape came from an import has one to repair, and only once;
    /// returns whether the request was recorded. Not undoable: the shape
    /// before the repair would have to be derived from the file again.
    pub fn request_body_repair(&mut self, body: BodyId) -> bool {
        let pending = self.body(body).is_some_and(|b| !b.repair_requested);
        if !pending || !(self.body_solid_is_imported(body) || self.has_base_solid(body)) {
            return false;
        }
        self.record_and_apply(op::DocumentOp::RequestBodyRepair { id: body });
        true
    }

    /// Read the body's shape from the file `name`, whose contents are
    /// `bytes`: its first solid becomes the body's base (what its history
    /// builds on), or its shape when it has no history. For a body whose
    /// shape comes from outside its history (an import, a converted mesh,
    /// a base); not undoable, as an import is not. Whether it was
    /// recorded.
    pub fn replace_body_shape(&mut self, body: BodyId, name: &str, bytes: Vec<u8>) -> bool {
        if !self.can_replace_shape(body) || bytes.is_empty() {
            return false;
        }
        self.record_and_apply(op::DocumentOp::ReplaceBodyShape {
            id: body,
            asset: shape_asset(name, "replaces_shape_of", body),
            bytes: op::BlobPayload(std::sync::Arc::new(bytes)),
        });
        true
    }

    /// Give a body that holds nothing yet its shape from the file `name`,
    /// whose contents are `bytes`, as [`Self::replace_body_shape`] reads
    /// one: a shape with no history. Whether it was recorded.
    pub fn shape_body_from_file(&mut self, body: BodyId, name: &str, bytes: Vec<u8>) -> bool {
        let empty = self.bodies.iter().any(|b| {
            b.id == body && b.link.is_none() && b.copy_of.is_none() && b.shape_asset.is_none()
        }) && !self.has_base_solid(body)
            && self.imported_geometry(body).is_none()
            && !self
                .feature_tree
                .all_nodes()
                .any(|(_, n)| n.body == Some(body));
        if !empty || bytes.is_empty() {
            return false;
        }
        self.record_and_apply(op::DocumentOp::ReplaceBodyShape {
            id: body,
            asset: shape_asset(name, "shape_of", body),
            bytes: op::BlobPayload(std::sync::Arc::new(bytes)),
        });
        true
    }

    /// Keep the shape in the file `name`, whose contents are `bytes`, to
    /// stand in for `body`'s solid while its history does not build up to
    /// `feature`, one of its features: read as an imported shape is, it
    /// shows in place of what the build left until a build succeeds. Not
    /// undoable, as an import is not. Whether it was recorded.
    pub fn set_body_stand_in(
        &mut self,
        body: BodyId,
        feature: FeatureId,
        name: &str,
        bytes: Vec<u8>,
    ) -> bool {
        let in_body = self
            .feature_tree
            .get_node(feature)
            .is_some_and(|n| n.body == Some(body));
        if !in_body || bytes.is_empty() {
            return false;
        }
        self.record_and_apply(op::DocumentOp::SetBodyStandIn {
            id: body,
            feature,
            asset: shape_asset(name, "stands_in_for", body),
            bytes: op::BlobPayload(std::sync::Arc::new(bytes)),
        });
        true
    }

    /// The asset of `body`'s stand-in when it is due: the feature it stands
    /// in for is the body's tip, that feature's build failed, and the body
    /// does not show the stand-in already.
    pub fn stand_in_due(&self, body: BodyId) -> Option<Uuid> {
        let stand_in = self.body(body)?.stand_in?;
        let tip = self.body_tip(body)?;
        let failed = self
            .feature_tree
            .get_node(stand_in.feature)
            .is_some_and(|n| n.error.is_some());
        let shown = self
            .imported_geometry(body)
            .is_some_and(|g| g.source_asset == Some(stand_in.asset));
        (tip == stand_in.feature && failed && !shown).then_some(stand_in.asset)
    }

    /// Whether `asset` is what stands in for `body`'s solid.
    pub fn is_stand_in(&self, body: BodyId, asset: Uuid) -> bool {
        self.body(body)
            .is_some_and(|b| b.stand_in.is_some_and(|s| s.asset == asset))
    }

    /// Whether the body's shape comes from a file (an import, a converted
    /// mesh, or its base), so another file can replace it: not a body
    /// built from its history alone, a linked copy or a linked part.
    pub fn can_replace_shape(&self, body: BodyId) -> bool {
        self.has_base_solid(body)
            || (self.copy_source(body).is_none()
                && self.body(body).is_some_and(|b| b.link.is_none())
                && self
                    .imported_geometry(body)
                    .is_some_and(|g| g.source_asset.is_some()))
    }

    /// Bodies whose shape is to be read from a new asset and has not been
    /// yet, each with the asset: the host derives the shape from it.
    pub fn bodies_awaiting_shape(&self) -> Vec<(BodyId, Uuid)> {
        self.bodies
            .iter()
            .filter_map(|b| {
                let asset = b.shape_asset?;
                let shape = self
                    .base_solids
                    .get(&b.id)
                    .or_else(|| self.imported_geometry(b.id));
                (shape.and_then(|g| g.source_asset) != Some(asset)).then_some((b.id, asset))
            })
            .chain(
                self.bodies
                    .iter()
                    .filter_map(|b| Some((b.id, self.stand_in_due(b.id)?))),
            )
            .collect()
    }

    /// Whether a body is a mesh from a mesh file, not yet a solid: it has
    /// triangles and no shape snapshot.
    pub fn is_mesh_body(&self, body: BodyId) -> bool {
        self.imported_geometry(body)
            .and_then(|g| g.source_asset)
            .and_then(|asset| self.get_asset(asset))
            .is_some_and(|asset| asset.asset_type.is_mesh())
            && self.imported_brep_blob(body).is_none()
    }

    /// Whether "Convert to solid" applies: a mesh body whose conversion
    /// was not asked for yet.
    pub fn can_convert(&self, body: BodyId) -> bool {
        self.is_mesh_body(body) && self.body(body).is_some_and(|b| !b.solid_requested)
    }

    /// Ask for a mesh body to become a B-rep solid; once, and only for a
    /// mesh body. Returns whether the request was recorded. Not undoable:
    /// the mesh it was would have to be derived from the file again.
    pub fn request_mesh_solid(&mut self, body: BodyId) -> bool {
        if !self.can_convert(body) {
            return false;
        }
        self.record_and_apply(op::DocumentOp::RequestMeshSolid { id: body });
        true
    }

    /// Mesh bodies whose conversion was asked for and has not landed: the
    /// host derives each one.
    pub fn bodies_awaiting_solid(&self) -> Vec<BodyId> {
        self.bodies
            .iter()
            .filter(|b| b.solid_requested && self.is_mesh_body(b.id))
            .map(|b| b.id)
            .collect()
    }

    /// Whether the body is a converted solid whose curved stretches are
    /// still facets, not yet sent to be refined, and takes no features yet:
    /// the faces a refine rebuilds are the ones features would stand on.
    pub fn can_refine(&self, body: BodyId) -> bool {
        self.body(body)
            .is_some_and(|b| !b.refine_requested && b.link.is_none())
            && !self.has_base_solid(body)
            && self.copy_source(body).is_none()
            && self.imported_geometry(body).is_some_and(|g| {
                g.source_asset.is_some() && g.health.as_ref().is_some_and(|h| h.faceted)
            })
    }

    /// Ask for a converted solid's facets to be rebuilt on the surfaces
    /// they approximate; once, and only where [`Self::can_refine`].
    /// Returns whether the request was recorded. Not undoable, as a
    /// conversion is not.
    pub fn request_body_refine(&mut self, body: BodyId) -> bool {
        if !self.can_refine(body) {
            return false;
        }
        self.record_and_apply(op::DocumentOp::RequestBodyRefine { id: body });
        true
    }

    /// Bodies whose refine was asked for and whose shape is still the
    /// facets: the host derives each one.
    pub fn bodies_awaiting_refine(&self) -> Vec<BodyId> {
        self.bodies
            .iter()
            .filter(|b| b.refine_requested)
            .filter(|b| {
                self.imported_geometry(b.id)
                    .and_then(|g| g.health.as_ref())
                    .is_some_and(|h| h.faceted)
            })
            .map(|b| b.id)
            .collect()
    }

    /// Bodies whose repair was asked for and whose geometry is not yet the
    /// repaired shape: the host derives each one.
    pub fn bodies_awaiting_repair(&self) -> Vec<BodyId> {
        self.bodies
            .iter()
            .filter(|b| b.repair_requested)
            .filter(|b| {
                // A body with a base mends its base: the shape its features
                // build on.
                let shape = self
                    .base_solids
                    .get(&b.id)
                    .or_else(|| self.imported_geometry(b.id));
                shape.is_some_and(|g| {
                    g.source_asset.is_some() && !g.health.as_ref().is_some_and(|h| h.repaired)
                })
            })
            .map(|b| b.id)
            .collect()
    }
}
