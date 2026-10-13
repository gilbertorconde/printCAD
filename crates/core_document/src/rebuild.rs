//! What a bench hands the host to rebuild a body's solid.

use kernel_api::{
    ChainProbe, ChainProjection, ProbeAnswer, ProjectedEdge, ProjectionProbe, ShapeProbe, SolidOp,
};

use crate::feature::{BodyId, FeatureId};

/// A body's build chain plus the feature responsible for each op (one
/// feature can emit several ops, e.g. a counterbored hole), and what the
/// features standing on the solid part way through ask of it.
#[derive(Debug, Default)]
pub struct BuildPlan {
    pub ops: Vec<SolidOp>,
    pub op_features: Vec<FeatureId>,
    /// Questions about the solid, each asked where its feature stands in
    /// the history (a datum on a face finding the face again).
    pub probes: Vec<PlanProbe>,
    /// Solids' edges and faces projected onto planes, each of the solid
    /// where its feature stands in the history (a sketch taking geometry
    /// from its own body's solid).
    pub projections: Vec<PlanProjection>,
    /// The feature the plan stops at, which could not be planned: the ops
    /// are the history before it, which the body shows.
    pub failed: Option<BuildError>,
    /// The features after a failure, left out of the build.
    pub unbuilt: Vec<FeatureId>,
}

/// One question a feature asks of its body's solid during a build.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlanProbe {
    pub feature: FeatureId,
    pub probe: ChainProbe,
}

/// One projection a feature asks of its body's solid during a build.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlanProjection {
    pub feature: FeatureId,
    pub projection: ChainProjection,
}

/// What a build projected for a feature: the projections it asked, in its
/// own order, and the curves each came to. Derived on each replica, never
/// an op.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProjectedReferences {
    pub asked: Vec<ProjectionProbe>,
    pub answers: Vec<Result<Vec<ProjectedEdge>, String>>,
}

impl ProjectedReferences {
    /// What projecting `probe` came to, when the build asked it.
    pub fn answer(&self, probe: &ProjectionProbe) -> Option<&Result<Vec<ProjectedEdge>, String>> {
        self.asked
            .iter()
            .position(|p| p == probe)
            .and_then(|at| self.answers.get(at))
    }
}

/// The projections of a build, by the feature that asked them.
pub fn sort_projections(
    asked: &[PlanProjection],
    answers: &[Result<Vec<ProjectedEdge>, String>],
) -> Vec<(FeatureId, ProjectedReferences)> {
    let mut sorted: Vec<(FeatureId, ProjectedReferences)> = Vec::new();
    for (asked, answer) in asked.iter().zip(answers) {
        let at = match sorted.iter().position(|(f, _)| *f == asked.feature) {
            Some(at) => at,
            None => {
                sorted.push((asked.feature, ProjectedReferences::default()));
                sorted.len() - 1
            }
        };
        sorted[at].1.asked.push(asked.projection.probe);
        sorted[at].1.answers.push(answer.clone());
    }
    sorted
}

/// What a build found of a feature's references: the probes it asked, in
/// its own order, and their answers. Derived on each replica, never an op.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProbedReferences {
    pub probes: Vec<ShapeProbe>,
    pub answers: Vec<Result<ProbeAnswer, String>>,
}

/// The answers of a build, by the feature whose probes asked them.
pub fn sort_answers(
    asked: &[PlanProbe],
    answers: &[Result<ProbeAnswer, String>],
) -> Vec<(FeatureId, ProbedReferences)> {
    let mut sorted: Vec<(FeatureId, ProbedReferences)> = Vec::new();
    for (asked, answer) in asked.iter().zip(answers) {
        let at = match sorted.iter().position(|(f, _)| *f == asked.feature) {
            Some(at) => at,
            None => {
                sorted.push((asked.feature, ProbedReferences::default()));
                sorted.len() - 1
            }
        };
        let references = &mut sorted[at].1;
        references.probes.push(asked.probe.probe);
        references.answers.push(answer.clone());
    }
    sorted
}

impl BuildPlan {
    /// Stop the plan at the first feature whose formula fails
    /// (`Document::build_formula_error`): it fails with the formula's
    /// error, as an unplannable feature does, rather than build from the
    /// number its data held before, and what follows it is left unbuilt.
    pub fn stop_at_failing_formulas(&mut self, document: &crate::Document) {
        let Some((at, feature, why)) = self
            .op_features
            .iter()
            .enumerate()
            .find_map(|(i, f)| document.build_formula_error(*f).map(|why| (i, *f, why)))
        else {
            return;
        };
        let mut unbuilt: Vec<FeatureId> = Vec::new();
        let earlier_failure = self.failed.take().and_then(|e| e.feature);
        for f in self.op_features[at..]
            .iter()
            .copied()
            .chain(earlier_failure)
            .chain(std::mem::take(&mut self.unbuilt))
        {
            if f != feature && !unbuilt.contains(&f) {
                unbuilt.push(f);
            }
        }
        self.ops.truncate(at);
        self.op_features.truncate(at);
        self.probes.retain(|p| p.probe.after_op <= at);
        self.projections.retain(|p| p.projection.after_op <= at);
        self.failed = Some(BuildError {
            feature: Some(feature),
            message: why,
        });
        self.unbuilt = unbuilt;
    }
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
