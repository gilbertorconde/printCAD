//! How a bench's features present in the tree and the view.

use super::{
    FeatureInfo, FeaturePart, FeaturePaths, FeatureStatus, PartPick, PassiveGeometry, ViewportPick,
    sole_dependency,
};
use crate::feature::{BodyId, FeatureNode};
use crate::runtime::WorkbenchRuntimeContext;
use crate::{Document, FeatureId};

/// The features of the kinds a bench claims: how each presents in the
/// tree, what it draws and offers to pick while not under edit, and its
/// edit session. The registry asks the owning bench whichever bench is
/// active.
pub trait BenchFeatures {
    /// How a feature of one of this bench's `feature_kinds` presents. The
    /// registry calls it on the owning bench whichever bench is active.
    fn feature_info(&self, node: &FeatureNode) -> FeatureInfo {
        FeatureInfo::fallback(node)
    }

    /// The feature an owned feature consumes, whose row it sits under in
    /// the tree (a pad under its sketch, a sketch under its datum). `None`
    /// puts it at its body's top level in build order: a feature that
    /// works on the whole body, or reads several features without
    /// consuming one. Unless a bench says otherwise, a feature consumes
    /// its one dependency and nothing when it has several.
    fn consumed_feature(&self, document: &Document, node: &FeatureNode) -> Option<FeatureId> {
        sole_dependency(document, node)
    }

    /// What the scene draws for an owned feature that is visible and not
    /// under edit. Called on the owner whichever bench is active; the host
    /// colours it and skips the feature under edit.
    fn passive_geometry(
        &self,
        _document: &Document,
        _id: FeatureId,
        _node: &FeatureNode,
    ) -> Option<PassiveGeometry> {
        None
    }

    /// The curves and points an owned feature lays out, for what is
    /// repeated along or at them (an array of copies along a path or at
    /// points). `None` when it lays out none.
    fn feature_paths(
        &self,
        _document: &Document,
        _id: FeatureId,
        _node: &FeatureNode,
    ) -> Option<FeaturePaths> {
        None
    }

    /// The bounded solid an owned feature offers a mesh boolean as its
    /// operand (a fitted cylinder over its drawn extent), in the
    /// document's frame. `None` when it offers none.
    fn feature_solid(
        &self,
        _document: &Document,
        _id: FeatureId,
    ) -> Option<kernel_api::MeshOperand> {
        None
    }

    /// The plane an owned feature offers to sketch on (a scan's section,
    /// a fitted plane), in the document's frame: Create sketch puts a new
    /// sketch there while the feature is selected. `None` when it offers
    /// none.
    fn feature_plane(
        &self,
        _document: &Document,
        _id: FeatureId,
        _node: &FeatureNode,
    ) -> Option<kernel_api::ProfilePlane> {
        None
    }

    /// A problem an owned feature has that does not stop it building (a
    /// sketch whose projected edges are gone), which the tree warns of on
    /// its row. Asked whenever the tree is drawn, so it answers from what
    /// it worked out last for the same inputs.
    fn feature_warning(
        &self,
        _document: &Document,
        _kernel: Option<&dyn kernel_api::KernelQueries>,
        _id: FeatureId,
    ) -> Option<String> {
        None
    }

    /// How far, in pixels, the cursor is from an owned feature, when it is
    /// close enough to count as over it. `None` when it is not.
    fn pick_feature(
        &self,
        _document: &Document,
        _id: FeatureId,
        _node: &FeatureNode,
        _pick: &ViewportPick,
    ) -> Option<f32> {
        None
    }

    /// The part of an owned feature under the cursor: a curve within
    /// `tolerance_px` of it, else the region it lies in. `None` when the
    /// feature has no parts to pick there. Asked only while no task or
    /// edit session is open.
    fn pick_part(
        &self,
        _document: &Document,
        _id: FeatureId,
        _node: &FeatureNode,
        _pick: &ViewportPick,
        _tolerance_px: f32,
    ) -> Option<PartPick> {
        None
    }

    /// The lines one part of an owned feature draws (a curve), in its
    /// body's frame, for the view to draw it picked. `None` for a part
    /// drawn some other way (a region, through `passive_geometry`).
    fn part_lines(
        &self,
        _document: &Document,
        _id: FeatureId,
        _part: &FeaturePart,
    ) -> Option<kernel_api::TriMesh> {
        None
    }

    /// How the work of a feature of a kind this bench claims stands, when
    /// the bench says.
    fn feature_status(&self, _node: &FeatureNode) -> Option<FeatureStatus> {
        None
    }

    /// A feature of a kind this bench claims, dragged in the tree and
    /// dropped just before (`before`) or after `target`: true when the
    /// bench took it (moved it its own way, or refused), false to leave the
    /// host to move it in its body's history.
    fn feature_dropped(
        &mut self,
        _ctx: &mut WorkbenchRuntimeContext,
        _feature: FeatureId,
        _target: FeatureId,
        _before: bool,
    ) -> bool {
        false
    }

    /// The feature whose edit session is open, for the tree to badge.
    fn editing_feature(&self) -> Option<FeatureId> {
        None
    }

    /// The user asked to edit `id`, a feature of a kind this bench claims
    /// (a double click on its tree row): open its task. Selecting a feature
    /// alone never opens one; the host has made `id` the active document
    /// object before this call.
    fn edit_feature(&mut self, _ctx: &mut WorkbenchRuntimeContext, _id: FeatureId) {}

    /// Finish/close the current editing session (e.g., finish sketch).
    /// Called when the user requests to finish editing (e.g., via UI button).
    fn finish_editing(&mut self, _ctx: &mut WorkbenchRuntimeContext) {}

    /// Features of other bodies that act on `body` (a joint holding
    /// another body to it): the tree lists each under `body` too, as a
    /// link to the feature.
    fn linked_features(&self, _document: &Document, _body: BodyId) -> Vec<FeatureId> {
        Vec::new()
    }

    /// Remove an owned feature and settle what depended on it: features
    /// it hid come back, its body rebuilds. `false` when nothing was
    /// removed.
    fn delete_feature(&mut self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) -> bool {
        ctx.document.remove_feature(id).is_ok()
    }
}
