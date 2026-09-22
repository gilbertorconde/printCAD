//! What a bench hands the host to rebuild a body's solid.

use kernel_api::SolidOp;

use crate::feature::{BodyId, FeatureId};

/// A body's build chain plus the feature responsible for each op (one
/// feature can emit several ops, e.g. a counterbored hole).
#[derive(Debug)]
pub struct BuildPlan {
    pub ops: Vec<SolidOp>,
    pub op_features: Vec<FeatureId>,
}

/// A translation failure attributed to the feature that caused it.
#[derive(Debug, Clone)]
pub struct BuildError {
    pub feature: Option<FeatureId>,
    pub message: String,
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// One body whose solid a bench wants rebuilt now. An empty plan means the
/// body has no history left: its derived solid goes, an imported one stays.
#[derive(Debug)]
pub struct RebuildJob {
    pub body: BodyId,
    pub plan: Result<BuildPlan, BuildError>,
}
