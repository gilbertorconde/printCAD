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

/// The history boundary before the feature responsible for a failed op.
/// The runner owns retry, cancellation and preview policy.
pub struct FailedPrefix<F> {
    pub start: usize,
    pub feature: F,
    pub unbuilt: Vec<F>,
}

impl<F: Copy + Eq> FailedPrefix<F> {
    pub fn at(features: &[F], op: usize) -> Option<Self> {
        let feature = *features.get(op)?;
        let start = features.iter().position(|f| *f == feature)?;
        let mut unbuilt = Vec::new();
        for f in &features[start..] {
            if *f != feature && !unbuilt.contains(f) {
                unbuilt.push(*f);
            }
        }
        Some(Self {
            start,
            feature,
            unbuilt,
        })
    }

    /// Questions at the prefix boundary remain valid.
    pub fn retain_questions(
        &self,
        probes: &mut Vec<PlanProbe>,
        projections: &mut Vec<PlanProjection>,
    ) {
        probes.retain(|p| p.probe.after_op <= self.start);
        projections.retain(|p| p.projection.after_op <= self.start);
    }
}

impl BuildPlan {
    /// Retain complete features before the failing op and their questions.
    pub fn keep_before_failure(&mut self, op: usize) -> Option<FailedPrefix<FeatureId>> {
        let prefix = FailedPrefix::at(&self.op_features, op)?;
        self.ops.truncate(prefix.start);
        self.op_features.truncate(prefix.start);
        prefix.retain_questions(&mut self.probes, &mut self.projections);
        Some(prefix)
    }

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
        self.keep_before_failure(at)
            .expect("a formula failure names a planned operation");
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

/// A mesh body's history as a bench plans it: the steps in order, each
/// from the feature at the same place in `features`. Built from the body's
/// mesh start ([`crate::Document::mesh_start`]) whenever it changes.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshPlan {
    pub body: crate::BodyId,
    pub ops: Vec<kernel_api::MeshOp>,
    pub features: Vec<crate::FeatureId>,
}

impl MeshPlan {
    /// An empty history restores the imported mesh without running a kernel.
    pub fn restored_start(
        &self,
        mesh: &kernel_api::TriMesh,
    ) -> Option<kernel_api::MeshChainAnswer> {
        self.ops.is_empty().then(|| kernel_api::MeshChainAnswer {
            mesh: mesh.clone(),
            ..Default::default()
        })
    }

    /// Land a completed history or attribute a runner failure to its first step.
    pub fn land(
        self,
        document: &mut crate::Document,
        answer: Result<kernel_api::MeshChainAnswer, String>,
    ) {
        match answer {
            Ok(answer) => document.store_mesh_build(self, answer),
            Err(why) => {
                if let Some(id) = self.features.first() {
                    document.set_feature_error(*id, Some(why));
                }
            }
        }
    }
}

/// Derived measurements and vertex provenance for one complete mesh plan.
#[derive(Debug, Clone)]
pub(crate) struct MeshResult {
    pub plan: MeshPlan,
    pub steps: Vec<kernel_api::MeshStep>,
    pub sources: Vec<Option<u32>>,
}

/// One body whose solid a bench wants rebuilt now. An empty plan means the
/// body has no history left: its derived solid goes, an imported one stays.
#[derive(Debug)]
pub struct RebuildJob {
    pub body: BodyId,
    pub plan: Result<BuildPlan, BuildError>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_keep_whole_features_and_boundary_questions() {
        let [a, b, c] = [1, 2, 3].map(|n| FeatureId(uuid::Uuid::from_u128(n)));
        for (op, start, feature, unbuilt) in [
            (0, 0, a, vec![b, c]),
            (1, 1, b, vec![c]),
            (2, 1, b, vec![c]),
            (3, 3, c, vec![]),
        ] {
            let mut plan = BuildPlan {
                ops: vec![SolidOp::Refine; 4],
                op_features: vec![a, b, b, c],
                probes: (0..=4)
                    .map(|after_op| PlanProbe {
                        feature: a,
                        probe: ChainProbe {
                            after_op,
                            probe: ShapeProbe::Mass,
                        },
                    })
                    .collect(),
                projections: (0..=4)
                    .map(|after_op| PlanProjection {
                        feature: a,
                        projection: ChainProjection {
                            after_op,
                            probe: ProjectionProbe::default(),
                        },
                    })
                    .collect(),
                ..Default::default()
            };
            let prefix = plan.keep_before_failure(op).unwrap();
            assert_eq!(
                (prefix.start, prefix.feature, prefix.unbuilt),
                (start, feature, unbuilt)
            );
            assert_eq!(plan.ops.len(), start);
            assert_eq!(plan.op_features.len(), start);
            assert_eq!(plan.probes.len(), start + 1);
            assert_eq!(plan.projections.len(), start + 1);
            assert_eq!(plan.probes.last().unwrap().probe.after_op, start);
            assert_eq!(plan.projections.last().unwrap().projection.after_op, start);
        }
        assert!(FailedPrefix::at(&[a], 1).is_none());
    }

    #[test]
    fn an_empty_mesh_history_restores_the_exact_start() {
        let plan = MeshPlan {
            body: BodyId::new(),
            ops: vec![],
            features: vec![],
        };
        let mesh = kernel_api::TriMesh {
            positions: vec![[1.0, 2.0, 3.0]; 2],
            ..Default::default()
        };
        let answer = plan.restored_start(&mesh).unwrap();
        assert_eq!(answer.mesh.positions, mesh.positions);
        assert_eq!(answer.mesh.indices, mesh.indices);
    }
}
