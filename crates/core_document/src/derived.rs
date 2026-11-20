//! A document's derived build state: what its builds found, kept per
//! replica and never recorded as an op: the bodies' meshes, shape
//! snapshots and face colours, each feature's errors and warnings, the
//! answers to its probes and projections, size limits, mesh build
//! results, cosmetic threads and feature solids.
//!
//! [`Derived`] holds it and answers the reads (`Document::derived`);
//! [`DerivedMut`] writes it (`Document::derived_mut`). Neither is a way
//! to edit the document: a user edit goes through a `Document` mutator,
//! which records exactly one op, and nothing reached through
//! `derived_mut` records one (debug builds assert it). The replicated
//! projection leaves all of it out, and `Document::clone` keeps it (a
//! save snapshot is a clone). Only the bodies' meshes are saved, under
//! the document's `imported_meshes` key, beside the snapshots and colours
//! the container keeps as entries of their own.

use std::collections::HashMap;
use std::sync::Arc;

use crate::feature::{BodyId, FeatureId};
use crate::{Document, ImportedGeometry, LocalGeometry, rebuild, threads};

/// What the last build said of one feature. Derived on each replica.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuildStatus {
    /// Why its build failed, or why it was not built.
    pub error: Option<String>,
    /// The edges of a refused fillet or chamfer, with the error.
    pub refused_edges: kernel_api::EdgeRefusal,
    /// What the last build that took it says of it: a reference found by
    /// position because its name was lost.
    pub warning: Option<String>,
    /// The last build found it, a feature that adds or removes material,
    /// left the solid as it was: what it did not do and why that may be
    /// (`OpWarning::unchanged`), its kind to be put before it.
    pub unchanged: Option<String>,
}

static NO_REFUSAL: kernel_api::EdgeRefusal = kernel_api::EdgeRefusal {
    request: Vec::new(),
    failing: Vec::new(),
    fits: None,
};

/// A document's derived build state. See the module notes.
#[derive(Debug, Clone, Default)]
pub struct Derived {
    /// Each body's mesh as the scene draws and picks it: placed where the
    /// body sits. Saved with the document (`saved_meshes`).
    pub(crate) imported_meshes: HashMap<BodyId, ImportedGeometry>,
    /// The meshes of placed bodies in their own frame, beside the placed
    /// copies in `imported_meshes`, with their bounds: rebuilt from the
    /// placed copy on load.
    pub(crate) local_meshes: HashMap<BodyId, LocalGeometry>,
    /// Bodies' shape snapshots (ogeom native text), saved as
    /// `brep/<body>.bin` beside the JSON rather than in it.
    pub(crate) imported_brep_blobs: HashMap<BodyId, Arc<Vec<u8>>>,
    /// Per-face RGB snapshot parallel to `imported_brep_blobs` face order.
    pub(crate) imported_brep_face_colors: HashMap<BodyId, Vec<[f32; 3]>>,
    /// Each feature's errors and warnings from its last build.
    pub(crate) status: HashMap<FeatureId, BuildStatus>,
    /// What each feature was last built against from outside its own
    /// history (another body's solid, where it sat), as its bench sums it
    /// (`BenchFeatures::built_against`).
    pub(crate) built_against: HashMap<FeatureId, u64>,
    /// What the last build found of each feature's references on its
    /// body's solid.
    pub(crate) probed: HashMap<FeatureId, rebuild::ProbedReferences>,
    /// What the last build projected for each feature from its body's
    /// solid.
    pub(crate) projected: HashMap<FeatureId, rebuild::ProjectedReferences>,
    /// The largest size each previewed fillet or chamfer builds at, as its
    /// last preview build found it.
    pub(crate) size_limits: HashMap<FeatureId, kernel_api::SizeLimit>,
    /// Mesh build measurements and source vertices, keyed by the full plan.
    pub(crate) mesh_results: HashMap<BodyId, rebuild::MeshResult>,
    /// The cosmetic threads the features in the bodies' builds declare,
    /// as the registry last worked them out.
    pub(crate) threads: Vec<threads::ThreadCallout>,
    /// The geometry revision the threads were worked out at: a copy of a
    /// threaded hole is threaded where the built solid shows its bore.
    pub(crate) threads_at: Option<u64>,
    /// The bounded solid each feature's owner offers
    /// (`BenchFeatures::feature_solid`), in the document's frame: derived at
    /// evaluation, read by mesh booleans.
    pub(crate) feature_solids: HashMap<FeatureId, kernel_api::MeshOperand>,
    /// The next geometry revision handed out: revisions never repeat in a
    /// document, even for a body whose geometry went and came back.
    pub(crate) next_geometry_revision: u64,
    /// How many times what was found moved what formulas read (a probe's
    /// or a projection's answer): it counts toward
    /// `Document::mutation_seq`, so the formulas are worked out again,
    /// without marking the document edited.
    pub(crate) moved: u64,
}

impl Derived {
    /// A body's mesh as the scene draws and picks it, placed where the
    /// body sits.
    pub fn imported_geometry(&self, body: BodyId) -> Option<&ImportedGeometry> {
        self.imported_meshes.get(&body)
    }

    /// Every body's mesh, placed.
    pub fn imported_geometries(&self) -> impl Iterator<Item = (&BodyId, &ImportedGeometry)> {
        self.imported_meshes.iter()
    }

    /// A body's geometry as its kernel shape has it, in the body's own
    /// frame: the mesh and its bounds before the body's placement. The
    /// scene's copy (`imported_geometry`) is placed.
    pub fn local_geometry(&self, body: BodyId) -> Option<LocalGeometry> {
        if let Some(local) = self.local_meshes.get(&body) {
            return Some(local.clone());
        }
        self.imported_meshes
            .get(&body)
            .map(|g| (Arc::clone(&g.mesh), g.bounds_mm))
    }

    /// A body's shape snapshot.
    pub fn imported_brep_blob(&self, body: BodyId) -> Option<&[u8]> {
        self.imported_brep_blobs.get(&body).map(|v| v.as_slice())
    }

    /// Shared handle to a body's shape snapshot; cloning is a refcount
    /// bump, so this is the cheap way to hand the blob to a worker thread.
    pub fn imported_brep_blob_arc(&self, body: BodyId) -> Option<Arc<Vec<u8>>> {
        self.imported_brep_blobs.get(&body).cloned()
    }

    /// The colour of each face of a body's shape snapshot, in its order.
    pub fn imported_brep_face_colors(&self, body: BodyId) -> Option<&[[f32; 3]]> {
        self.imported_brep_face_colors
            .get(&body)
            .map(|v| v.as_slice())
    }

    /// What the last build said of `feature`, when it said anything.
    pub fn status(&self, feature: FeatureId) -> Option<&BuildStatus> {
        self.status.get(&feature)
    }

    /// Why `feature`'s last build failed, or why it was not built.
    pub fn error(&self, feature: FeatureId) -> Option<&str> {
        self.status(feature)?.error.as_deref()
    }

    /// What `feature`'s last build warns of.
    pub fn warning(&self, feature: FeatureId) -> Option<&str> {
        self.status(feature)?.warning.as_deref()
    }

    /// What `feature` left undone in its last build, when it changed
    /// nothing.
    pub fn unchanged(&self, feature: FeatureId) -> Option<&str> {
        self.status(feature)?.unchanged.as_deref()
    }

    /// The edges `feature`'s last build refused, when it is a fillet or
    /// chamfer that failed; empty otherwise.
    pub fn refused_edges(&self, feature: FeatureId) -> &kernel_api::EdgeRefusal {
        self.status(feature)
            .map_or(&NO_REFUSAL, |status| &status.refused_edges)
    }

    /// Record (or clear) a recompute error on a feature, which drops the
    /// edges a refused fillet or chamfer kept with it.
    pub fn set_feature_error(&mut self, feature: FeatureId, error: Option<String>) {
        let status = self.status.entry(feature).or_default();
        status.error = error;
        status.refused_edges = Default::default();
    }

    /// Record a recompute error with the edges of the refused fillet or
    /// chamfer it comes from.
    pub fn set_feature_failure(
        &mut self,
        feature: FeatureId,
        error: String,
        edges: kernel_api::EdgeRefusal,
    ) {
        let status = self.status.entry(feature).or_default();
        status.error = Some(error);
        status.refused_edges = edges;
    }

    /// Clear `feature`'s errors and warnings.
    pub(crate) fn clear_status(&mut self, feature: FeatureId) {
        self.status.remove(&feature);
    }

    /// Keep a build's warnings on the features they are about: an op's on
    /// the feature `op_features` names for it, a probe's or projection's
    /// on the feature that asked it (a datum on a face, a sketch on one or
    /// taking its edges). Several on one feature are joined; one saying the
    /// feature changed nothing is kept apart, as [`BuildStatus::unchanged`].
    pub fn store_build_warnings(
        &mut self,
        op_features: &[FeatureId],
        probes: &[rebuild::PlanProbe],
        projections: &[rebuild::PlanProjection],
        result: &kernel_api::SolidBuildResult,
    ) {
        let owner = |features: Vec<FeatureId>, warnings: &[kernel_api::OpWarning]| {
            warnings
                .iter()
                .filter_map(|w| Some((*features.get(w.op_index)?, w.clone())))
                .collect::<Vec<_>>()
        };
        let warned = [
            owner(op_features.to_vec(), &result.warnings),
            owner(
                probes.iter().map(|p| p.feature).collect(),
                &result.probe_warnings,
            ),
            owner(
                projections.iter().map(|p| p.feature).collect(),
                &result.projection_warnings,
            ),
        ];
        for (id, warning) in warned.into_iter().flatten() {
            let status = self.status.entry(id).or_default();
            if warning.unchanged {
                status.unchanged = Some(warning.message.clone());
            } else {
                let message = &warning.message;
                status.warning = Some(match status.warning.take() {
                    Some(kept) if kept.contains(message.as_str()) => kept,
                    Some(kept) => format!("{kept}; {message}"),
                    None => message.to_string(),
                });
            }
        }
    }

    /// Keep the largest size the build of `op_features` found for its
    /// previewed fillet or chamfer, under the feature it belongs to.
    pub fn store_size_limit(
        &mut self,
        op_features: &[FeatureId],
        result: &kernel_api::SolidBuildResult,
    ) {
        if let Some(limit) = result.size_limit
            && let Some(feature) = op_features.get(limit.op_index)
        {
            self.size_limits.insert(*feature, limit);
        }
    }

    /// The largest size `feature`'s last preview build found it builds at,
    /// with the key of the op it was found for.
    pub fn size_limit(&self, feature: FeatureId) -> Option<&kernel_api::SizeLimit> {
        self.size_limits.get(&feature)
    }

    /// What `feature` was last built against from outside its own history,
    /// as noted by [`Self::note_built_against`].
    pub fn built_against(&self, feature: FeatureId) -> Option<u64> {
        self.built_against.get(&feature).copied()
    }

    /// Note what `feature` is being built against now (a summary its bench
    /// computes, such as another body's geometry revision and placement),
    /// so a later change to it can be told.
    pub fn note_built_against(&mut self, feature: FeatureId, inputs: u64) {
        self.built_against.insert(feature, inputs);
    }

    /// What the last build found of `feature`'s references on its body's
    /// solid.
    pub fn probed_references(&self, feature: FeatureId) -> Option<&rebuild::ProbedReferences> {
        self.probed.get(&feature)
    }

    /// Keep what a build found of `feature`'s references. When it differs
    /// from what was kept, the formulas and what follows from them are
    /// worked out again; the document is not marked edited.
    pub fn set_probed_references(
        &mut self,
        feature: FeatureId,
        references: rebuild::ProbedReferences,
    ) {
        if self.probed.get(&feature) != Some(&references) {
            self.probed.insert(feature, references);
            self.moved = self.moved.wrapping_add(1);
        }
    }

    /// Keep what a build found of its features' references (`answers` to
    /// `asked`, in order). A feature whose reference was not found carries
    /// the reason as its error, and keeps its references as they were.
    pub fn store_probe_answers(
        &mut self,
        asked: &[rebuild::PlanProbe],
        answers: &[Result<kernel_api::ProbeAnswer, String>],
    ) {
        for (feature, references) in rebuild::sort_answers(asked, answers) {
            if let Some(Err(e)) = references.answers.iter().find(|a| a.is_err()) {
                self.set_feature_error(
                    feature,
                    Some(format!("what it stands on is not found: {e}")),
                );
            }
            self.set_probed_references(feature, references);
        }
    }

    /// What the last build projected for `feature` from its body's solid.
    pub fn projected_references(
        &self,
        feature: FeatureId,
    ) -> Option<&rebuild::ProjectedReferences> {
        self.projected.get(&feature)
    }

    /// Keep what a build projected for its features (`answers` to `asked`,
    /// in order). When it differs from what was kept, the formulas and
    /// what follows from them are worked out again; the document is not
    /// marked edited. A projection that failed stays with its reason, for
    /// the feature to report.
    pub fn store_projections(
        &mut self,
        asked: &[rebuild::PlanProjection],
        answers: &[Result<Vec<kernel_api::ProjectedEdge>, String>],
    ) {
        for (feature, references) in rebuild::sort_projections(asked, answers) {
            if self.projected.get(&feature) != Some(&references) {
                self.projected.insert(feature, references);
                self.moved = self.moved.wrapping_add(1);
            }
        }
    }

    /// Measurements only while the complete history prefix still agrees.
    pub fn mesh_steps(&self, plan: &rebuild::MeshPlan) -> Option<&[kernel_api::MeshStep]> {
        let built = self.mesh_results.get(&plan.body)?;
        (&built.plan == plan).then_some(&built.steps)
    }

    /// Start vertices behind the displayed mesh of this exact plan.
    pub fn mesh_sources(&self, plan: &rebuild::MeshPlan) -> Option<&[Option<u32>]> {
        let built = self.mesh_results.get(&plan.body)?;
        (&built.plan == plan).then_some(&built.sources)
    }

    /// Every cosmetic thread the features in the bodies' builds declare
    /// (`BenchOutput::cosmetic_threads`), as last evaluated.
    pub fn cosmetic_threads(&self) -> &[threads::ThreadCallout] {
        &self.threads
    }

    /// The cosmetic threads of `body`, in history order.
    pub fn body_threads(&self, body: BodyId) -> impl Iterator<Item = &threads::ThreadCallout> {
        self.threads.iter().filter(move |t| t.body == body)
    }

    /// Take `threads` as the document's cosmetic threads.
    pub fn set_cosmetic_threads(&mut self, threads: Vec<threads::ThreadCallout>) {
        self.threads = threads;
        self.threads_at = Some(self.next_geometry_revision);
    }

    /// Whether a body's geometry changed since the threads were worked out.
    pub fn cosmetic_threads_stale(&self) -> bool {
        self.threads_at != Some(self.next_geometry_revision)
    }

    /// The bounded solid a feature offers a mesh boolean, in the
    /// document's frame.
    pub fn feature_solid(&self, id: FeatureId) -> Option<&kernel_api::MeshOperand> {
        self.feature_solids.get(&id)
    }

    /// Take `solids` as what each feature offers a mesh boolean.
    pub fn set_feature_solids(&mut self, solids: HashMap<FeatureId, kernel_api::MeshOperand>) {
        self.feature_solids = solids;
    }
}

/// The handle that writes a document's derived build state
/// (`Document::derived_mut`). It reads and writes [`Derived`] as its
/// target; what needs the rest of the document (the features of a body,
/// a body's placement and copies) is on the handle itself. Nothing done
/// through it records an op: debug builds assert it.
pub struct DerivedMut<'a> {
    pub(crate) doc: &'a mut Document,
}

impl<'a> DerivedMut<'a> {
    pub(crate) fn new(doc: &'a mut Document) -> Self {
        debug_assert!(!doc.deriving, "derived_mut taken twice");
        doc.deriving = true;
        Self { doc }
    }

    /// Clear recompute errors and warnings on every feature of a body (a
    /// rebuild is starting; its result will flag them again).
    pub fn clear_body_feature_errors(&mut self, body: BodyId) {
        let ids: Vec<FeatureId> = self
            .doc
            .feature_tree
            .all_nodes()
            .filter(|(_, n)| n.body == Some(body))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.doc.derived.clear_status(id);
        }
    }

    /// Store a body's geometry, given in the body's own frame; the scene's
    /// copy is placed where the body sits, and the body's linked copies
    /// take it. The geometry's `revision` is overwritten with the next one
    /// the document hands out, so a renderer tells a replaced mesh from
    /// last frame's with a cheap comparison. Marks the document edited
    /// (a build's caller puts back its clean state).
    pub fn set_imported_geometry(&mut self, body: BodyId, geometry: ImportedGeometry) {
        self.doc.set_imported_geometry(body, geometry);
    }

    /// Drop a body's built or imported geometry (mesh, shape snapshot,
    /// face colours): its history builds no solid.
    pub fn remove_imported_geometry(&mut self, body: BodyId) {
        self.doc.remove_imported_geometry(body);
    }

    /// Store a body's shape snapshot and face colours (in memory until
    /// save). An empty snapshot is no shape, as a mesh body has none: it
    /// clears the body's snapshot rather than storing one that describes
    /// nothing.
    pub fn set_imported_brep_data(
        &mut self,
        body: BodyId,
        brep_blob: Vec<u8>,
        face_colors: Vec<[f32; 3]>,
    ) {
        self.doc
            .set_imported_brep_data(body, brep_blob, face_colors);
    }

    /// Replace the body's base solid with what a repair (or another
    /// derivation of the same op) made of it, in the body's own frame.
    pub fn set_base_solid(
        &mut self,
        body: BodyId,
        geometry: ImportedGeometry,
        brep_blob: Vec<u8>,
        face_colors: Vec<[f32; 3]>,
    ) {
        self.doc
            .set_base_solid(body, geometry, brep_blob, face_colors);
    }

    /// Give a linked part its shape as read from its file: the mesh in its
    /// own frame, the snapshot and its face colours. Not an edit.
    pub fn set_linked_geometry(
        &mut self,
        body: BodyId,
        geometry: ImportedGeometry,
        blob: Option<Arc<Vec<u8>>>,
        colors: Option<Vec<[f32; 3]>>,
    ) {
        self.doc.set_linked_geometry(body, geometry, blob, colors);
    }

    /// A mirrored copy's snapshot, as the kernel made it from `from` (the
    /// source's snapshot then): kept when the source still has that one.
    pub fn set_mirrored_shape(&mut self, copy: BodyId, from: &Arc<Vec<u8>>, blob: Vec<u8>) -> bool {
        self.doc.set_mirrored_shape(copy, from, blob)
    }

    /// Land a mesh history's derived shape, measurements and feature errors.
    pub fn store_mesh_build(
        &mut self,
        plan: rebuild::MeshPlan,
        answer: kernel_api::MeshChainAnswer,
    ) {
        let Some(start) = self.doc.mesh_start(plan.body).cloned() else {
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
        let bounds_mm = answer.mesh.bounds();
        let body = plan.body;
        self.mesh_results.insert(
            body,
            rebuild::MeshResult {
                plan,
                steps: answer.steps,
                sources: answer.sources,
            },
        );
        self.doc.set_imported_geometry(
            body,
            ImportedGeometry {
                mesh: std::sync::Arc::new(answer.mesh),
                bounds_mm,
                mesh_path: None,
                ..start
            },
        );
    }

    /// Mark `feature` for rebuilding because something it is built from
    /// moved, without marking the document edited.
    pub fn mark_feature_stale(&mut self, feature: FeatureId) {
        self.doc.feature_tree.mark_dirty(feature);
    }
}

impl std::ops::Deref for DerivedMut<'_> {
    type Target = Derived;
    fn deref(&self) -> &Derived {
        &self.doc.derived
    }
}

impl std::ops::DerefMut for DerivedMut<'_> {
    fn deref_mut(&mut self) -> &mut Derived {
        &mut self.doc.derived
    }
}

impl Drop for DerivedMut<'_> {
    fn drop(&mut self) {
        self.doc.deriving = false;
    }
}

/// Writes [`Derived`] as the bodies' meshes alone, under the document's
/// `imported_meshes` key, and reads it back from them: what a reload
/// needs without tessellating again. The rest is derived again.
pub(crate) mod saved_meshes {
    use std::collections::HashMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::Derived;

    pub fn serialize<S: Serializer>(derived: &Derived, serializer: S) -> Result<S::Ok, S::Error> {
        derived.imported_meshes.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Derived, D::Error> {
        Ok(Derived {
            imported_meshes: HashMap::deserialize(deserializer)?,
            ..Derived::default()
        })
    }
}
