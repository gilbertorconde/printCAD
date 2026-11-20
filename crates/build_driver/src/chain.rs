//! Running a body's chain on the kernel.

use core_document::rebuild::FailedPrefix;
use core_document::{BuildPlan, FeatureId, PlanProbe, PlanProjection};
use kernel_api::{ChainError, SolidBuildResult, SolidOp, TessellationSettings};
use kernel_ogeom::{ChainCache, ChainRequest, OgeomKernel};

/// What one chain run builds: the ops, the feature of each, the detail,
/// the edited feature's ops and the probes and projections asked.
#[derive(Clone)]
pub struct Chain<'a> {
    pub ops: &'a [SolidOp],
    pub op_features: &'a [FeatureId],
    pub detail: &'a TessellationSettings,
    /// The edited feature's ops, whose preview the result carries.
    pub range: Option<std::ops::Range<usize>>,
    pub probes: &'a [PlanProbe],
    pub projections: &'a [PlanProjection],
}

impl<'a> Chain<'a> {
    /// The whole of `plan`, meshed at `detail`, without a preview.
    pub fn of_plan(plan: &'a BuildPlan, detail: &'a TessellationSettings) -> Self {
        Self {
            ops: &plan.ops,
            op_features: &plan.op_features,
            detail,
            range: None,
            probes: &plan.probes,
            projections: &plan.projections,
        }
    }

    /// The ops of `feature`, first to last, when the chain builds it.
    pub fn ops_of(op_features: &[FeatureId], feature: FeatureId) -> Option<std::ops::Range<usize>> {
        let first = op_features.iter().position(|f| *f == feature)?;
        let last = op_features.iter().rposition(|f| *f == feature)?;
        Some(first..last + 1)
    }
}

/// Run `chain` on `kernel`, resuming from the states `cache` keeps when
/// given one. Each op's faces are named after the feature it builds.
pub fn execute(
    kernel: &mut OgeomKernel,
    chain: &Chain<'_>,
    cache: Option<&mut ChainCache>,
) -> Result<SolidBuildResult, ChainError> {
    let asked: Vec<kernel_api::ChainProbe> = chain.probes.iter().map(|p| p.probe).collect();
    let projections: Vec<kernel_api::ChainProjection> =
        chain.projections.iter().map(|p| p.projection).collect();
    let tags: Vec<kernel_api::TopoName> = chain
        .op_features
        .iter()
        .map(|f| kernel_api::naming::name_of_id(f.0.as_bytes()))
        .collect();
    kernel.execute_solid_chain(
        ChainRequest::new(chain.ops, chain.detail)
            .tags(&tags)
            .preview(chain.range.clone())
            .probes(&asked)
            .projections(&projections)
            .cache(cache),
    )
}

/// Where a body's build stopped, and what it left out.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BuildFailure {
    /// The feature of the failing op, when the op has one.
    pub feature: Option<FeatureId>,
    /// The failing op's place in the chain.
    pub op_index: usize,
    pub error: String,
    /// The edges of the failing fillet or chamfer.
    pub edges: kernel_api::EdgeRefusal,
    /// The features after the failing one, left out.
    pub unbuilt: Vec<FeatureId>,
}

impl BuildFailure {
    /// A failure the kernel's cancel caused, not the history.
    pub fn cancelled(&self) -> bool {
        self.error.contains("cancelled")
    }
}

/// What a body's chain built.
pub enum Built<R> {
    /// The whole chain.
    Whole(R),
    /// The history before the failing feature: its first `ops` ops, with
    /// the probes and projections asked of them.
    Before {
        result: R,
        ops: usize,
        probes: Vec<PlanProbe>,
        projections: Vec<PlanProjection>,
        failure: BuildFailure,
    },
    /// Nothing: the first feature failed (`nothing_built`), the build was
    /// cancelled, or the history before the failure failed too.
    Failed {
        failure: BuildFailure,
        nothing_built: bool,
    },
}

/// Build `chain` with `run`; when an op fails, build the history before
/// the feature it belongs to, which is what the body shows. A cancelled
/// build stops there.
pub fn build_until_failure<R>(
    chain: Chain<'_>,
    mut run: impl FnMut(Chain<'_>) -> Result<R, ChainError>,
) -> Built<R> {
    let err = match run(chain.clone()) {
        Ok(result) => return Built::Whole(result),
        Err(err) => err,
    };
    let prefix = FailedPrefix::at(chain.op_features, err.op_index);
    let failure = BuildFailure {
        feature: prefix.as_ref().map(|p| p.feature),
        op_index: err.op_index,
        error: err.message,
        edges: err.edges,
        unbuilt: prefix.as_ref().map_or_else(Vec::new, |p| p.unbuilt.clone()),
    };
    let cancelled = failure.cancelled();
    let Some(prefix) = prefix.filter(|p| p.start > 0 && !cancelled) else {
        let nothing_built = !cancelled
            && FailedPrefix::at(chain.op_features, failure.op_index).is_some_and(|p| p.start == 0);
        return Built::Failed {
            failure,
            nothing_built,
        };
    };
    let mut probes = chain.probes.to_vec();
    let mut projections = chain.projections.to_vec();
    prefix.retain_questions(&mut probes, &mut projections);
    let start = prefix.start;
    match run(Chain {
        ops: &chain.ops[..start],
        op_features: &chain.op_features[..start],
        detail: chain.detail,
        range: None,
        probes: &probes,
        projections: &projections,
    }) {
        Ok(result) => Built::Before {
            result,
            ops: start,
            probes,
            projections,
            failure,
        },
        Err(_) => Built::Failed {
            failure,
            nothing_built: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failing_at(op: usize) -> ChainError {
        ChainError {
            op_index: op,
            message: "refused".into(),
            ..ChainError::default()
        }
    }

    /// A step failing part way builds the history before its feature
    /// (all of that feature's ops left out), and names the features after
    /// it as left out; a first feature failing builds nothing.
    #[test]
    fn a_failing_step_builds_the_history_before_its_feature() {
        let [a, b, c] = [1u128, 2, 3].map(|n| FeatureId(uuid::Uuid::from_u128(n)));
        let cube = SolidOp::Primitive {
            kind: kernel_api::PrimitiveKind::Box {
                length: 1.0,
                width: 1.0,
                height: 1.0,
            },
            placement: kernel_api::Placement::default(),
            op: kernel_api::BooleanOp::NewSolid,
        };
        let ops = vec![cube; 4];
        let features = [a, b, b, c];
        let detail = TessellationSettings::default();
        let plan = BuildPlan {
            ops: ops.clone(),
            op_features: features.to_vec(),
            ..BuildPlan::default()
        };
        let mut runs = Vec::new();
        let built = build_until_failure(Chain::of_plan(&plan, &detail), |chain| {
            runs.push(chain.ops.len());
            if chain.ops.len() > 2 {
                Err(failing_at(2))
            } else {
                Ok(chain.ops.len())
            }
        });
        assert_eq!(runs, [4, 1]);
        let Built::Before {
            result, failure, ..
        } = built
        else {
            panic!("the history before the failing feature builds");
        };
        assert_eq!(result, 1);
        assert_eq!(failure.feature, Some(b));
        assert_eq!(failure.unbuilt, [c]);

        let built = build_until_failure(Chain::of_plan(&plan, &detail), |_| {
            Err::<(), _>(failing_at(0))
        });
        assert!(matches!(
            built,
            Built::Failed {
                nothing_built: true,
                ..
            }
        ));
        let built = build_until_failure(Chain::of_plan(&plan, &detail), |_| {
            Err::<(), _>(ChainError {
                message: "cancelled".into(),
                ..failing_at(3)
            })
        });
        assert!(matches!(
            built,
            Built::Failed {
                nothing_built: false,
                ..
            }
        ));
    }
}
