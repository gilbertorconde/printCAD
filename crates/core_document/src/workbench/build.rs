//! How a bench's features turn into solids and meshes.

use super::*;

/// Building the bodies: the rebuild plans a bench hands the kernel, the
/// mesh histories it keeps, and the working data its features bring up
/// to date as the document evaluates.
pub trait BenchBuild {
    /// Make an owned feature's data whole once formulas' values are in it
    /// (a sketch solves for its new dimensions). `values` is the data the
    /// feature builds from; it is derived, and never recorded.
    fn settle(&self, _node: &FeatureNode, _values: &mut serde_json::Value) {}

    /// Bring an owned feature's working data up to date with the features
    /// it follows (a sketch takes the plane of the datum it was drawn on),
    /// before it settles. `values` is what formulas made of its data;
    /// `values_of` gives another feature's working data the same way;
    /// `document` the rest (where bodies sit). Answers whether it changed
    /// anything. Derived, never recorded, so it follows every change of
    /// what it follows, undo included.
    fn derive(
        &self,
        _node: &FeatureNode,
        _values: &mut serde_json::Value,
        _values_of: &dyn Fn(FeatureId) -> Option<serde_json::Value>,
        _document: &Document,
    ) -> bool {
        false
    }

    /// Bring an owned feature's working data up to date with what the last
    /// build found of the solid it stands on: `probed` answers the probes
    /// a plan asked for it (a sketch placed on a face asks where the face
    /// is now). Answers whether it changed anything. Derived, never
    /// recorded.
    fn derive_on_solid(
        &self,
        _node: &FeatureNode,
        _values: &mut serde_json::Value,
        _probed: &crate::rebuild::ProbedReferences,
    ) -> bool {
        false
    }

    /// Bring an owned feature's working data up to date with the bodies'
    /// geometry as it stands (a joint finds its faces again, by name, on a
    /// body rebuilt; a sketch projects solids' edges again). `kernel`
    /// answers what it asks of the solids, when the host has one. Runs
    /// after `derive` and `derive_on_solid`, so it sees where those put
    /// the feature. Answers whether it changed anything. Derived, never
    /// recorded.
    fn derive_on_geometry(
        &self,
        _node: &FeatureNode,
        _values: &mut serde_json::Value,
        _document: &Document,
        _kernel: Option<&dyn kernel_api::KernelQueries>,
    ) -> bool {
        false
    }

    /// The mesh histories this bench keeps on mesh bodies, as they stand:
    /// asked each frame, built by the host from each body's mesh start
    /// when a plan changes.
    fn mesh_plans(&self, _document: &Document) -> Vec<crate::MeshPlan> {
        Vec::new()
    }

    /// The bodies whose derived solid this bench must rebuild now, each
    /// with its plan. Called on every bench each frame. The bench settles
    /// the dirty flags of every feature a plan consumed before returning,
    /// or the same job comes back every frame.
    fn rebuild_jobs(&self, _document: &mut Document) -> Vec<RebuildJob> {
        Vec::new()
    }

    /// The body's history changed shape (a feature left it, its tip
    /// moved): rebuild it from the start, or drop its derived solid when
    /// no history is left.
    fn invalidate_body(&self, _document: &mut Document, _body: BodyId) {}

    /// Every derived solid this bench produces is stale.
    fn invalidate_all(&self, _document: &mut Document) {}
}
