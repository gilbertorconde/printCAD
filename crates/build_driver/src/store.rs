//! What a build leaves in the document. All of it is derived, per-replica
//! state: none of these record an op.

use core_document::{
    BodyId, BuildError, BuildPlan, Document, DocumentService, FeatureId, ImportedGeometry,
    PlanProbe, PlanProjection, RebuildJob,
};
use kernel_api::SolidBuildResult;

use crate::BuildFailure;

/// A body's job, started: what is left to build.
#[derive(Debug)]
pub struct Begun {
    pub body: BodyId,
    pub next: Next,
    /// The feature the plan could not plan, recorded on it.
    pub refused: Option<BuildError>,
}

/// What a started job asks of the kernel.
#[derive(Debug)]
pub enum Next {
    /// The plan's ops, never empty.
    Build(BuildPlan),
    /// The history builds nothing: the body's built geometry is dropped.
    Emptied,
    /// Nothing could be planned; the body keeps what it shows.
    Unplanned,
}

/// Start `job`: the body's errors and warnings are cleared, a feature the
/// plan could not plan carries the error and each feature after it a word
/// that it was not built, and a history that builds nothing drops the
/// geometry features made (an imported solid outlives it).
pub fn begin(document: &mut Document, job: RebuildJob) -> Begun {
    let body = job.body;
    document.derived_mut().clear_body_feature_errors(body);
    let (next, refused) = match job.plan {
        Ok(mut plan) => {
            let refused = plan.failed.take();
            if let Some(error) = &refused {
                refuse(document, error, &std::mem::take(&mut plan.unbuilt));
            }
            if plan.ops.is_empty() {
                if !document.body_solid_is_imported(body) {
                    document.derived_mut().remove_imported_geometry(body);
                }
                (Next::Emptied, refused)
            } else {
                (Next::Build(plan), refused)
            }
        }
        Err(error) => {
            refuse(document, &error, &[]);
            (Next::Unplanned, Some(error))
        }
    };
    Begun {
        body,
        next,
        refused,
    }
}

fn refuse(document: &mut Document, error: &BuildError, unbuilt: &[FeatureId]) {
    if let Some(feature) = error.feature {
        document
            .derived_mut()
            .set_feature_error(feature, Some(error.message.clone()));
        mark_unbuilt(document, feature, unbuilt.iter().copied());
    }
}

/// Keep what a build of `op_features` found besides its solid: the ops'
/// warnings, a previewed fillet's size limit, the probes' answers and the
/// projections.
pub fn store_answers(
    document: &mut Document,
    op_features: &[FeatureId],
    probes: &[PlanProbe],
    projections: &[PlanProjection],
    result: &SolidBuildResult,
) {
    let mut derived = document.derived_mut();
    derived.store_build_warnings(op_features, probes, projections, result);
    derived.store_size_limit(op_features, result);
    derived.store_probe_answers(probes, &result.probes);
    derived.store_projections(projections, &result.projections);
}

/// Keep the size limit read after the build of `feature`'s body, unless a
/// later build of it awaits another reading or none. Whether it was kept.
pub fn store_read_limit(
    document: &mut Document,
    feature: FeatureId,
    reading: u64,
    limit: kernel_api::SizeLimit,
) -> bool {
    document
        .derived_mut()
        .store_read_size_limit(feature, reading, limit)
}

/// Put a body's built solid in the document: its shape and the mesh drawn
/// from it. The solid is derived from the history, so landing it is not an
/// edit: a document opened and built again stays unedited.
pub fn store_solid(document: &mut Document, body: BodyId, result: SolidBuildResult) {
    let edited = document.metadata().dirty();
    let bounds_mm = result.bounds_mm;
    document
        .derived_mut()
        .set_imported_brep_data(body, result.brep_blob, Vec::new());
    document.derived_mut().set_imported_geometry(
        body,
        ImportedGeometry {
            mesh: std::sync::Arc::new(result.mesh),
            source_asset: None,
            revision: 0,
            bounds_mm,
            brep_blob_path: None,
            mesh_path: None,
            face_colors_path: None,
            health: None,
        },
    );
    if !edited {
        document.mark_clean();
    }
}

/// Pin a build's failure on its feature, with the edges a fillet or
/// chamfer refused, and on each feature after it a word that it was not
/// built.
pub fn record_failure(document: &mut Document, failure: &BuildFailure) {
    if let Some(feature) = failure.feature {
        document.derived_mut().set_feature_failure(
            feature,
            failure.error.clone(),
            failure.edges.clone(),
        );
        mark_unbuilt(document, feature, failure.unbuilt.iter().copied());
    }
}

/// The body's first feature failed: nothing of its history builds, and a
/// shape left from before would be stale. An imported solid stays.
pub fn clear_unbuilt(document: &mut Document, body: BodyId) {
    if !document.body_solid_is_imported(body) {
        document.derived_mut().remove_imported_geometry(body);
    }
}

/// Mark the features left out of a build because `failed`, before them,
/// fails.
fn mark_unbuilt(
    document: &mut Document,
    failed: FeatureId,
    unbuilt: impl IntoIterator<Item = FeatureId>,
) {
    let name = document
        .get_feature_meta(failed)
        .map(|n| n.name.clone())
        .unwrap_or_default();
    let mut derived = document.derived_mut();
    for feature in unbuilt {
        if feature != failed {
            derived.set_feature_error(
                feature,
                Some(format!("not built: `{name}` before it fails")),
            );
        }
    }
}

/// Where a shape read from a file went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landed {
    /// The body's base or its shape.
    Shape,
    /// The body's stand-in, shown while its history does not build.
    StandIn,
    /// Nothing: a stand-in no longer due.
    Dropped,
}

/// Put a shape read from the file `asset` on `body`: as its stand-in while
/// that is due, else as its base, which its features then build on, or as
/// its shape when it has no history.
pub fn land_shape(
    document: &mut Document,
    registry: &DocumentService,
    body: BodyId,
    asset: uuid::Uuid,
    read: kernel_api::MeshSolidResult,
) -> Landed {
    let geometry = ImportedGeometry {
        bounds_mm: read.bounds_mm.or_else(|| read.mesh.bounds()),
        mesh: std::sync::Arc::new(read.mesh),
        source_asset: Some(asset),
        revision: 0,
        brep_blob_path: None,
        mesh_path: None,
        face_colors_path: None,
        health: Some(read.health),
    };
    if document.is_stand_in(body, asset) {
        // A build that succeeded, or a tip moved, while it was read leaves
        // nothing to stand in for.
        if document.stand_in_due(body) != Some(asset) {
            return Landed::Dropped;
        }
        document.derived_mut().set_imported_geometry(body, geometry);
        document
            .derived_mut()
            .set_imported_brep_data(body, read.brep_blob, read.face_colors);
        return Landed::StandIn;
    }
    if document.has_base_solid(body) {
        document
            .derived_mut()
            .set_base_solid(body, geometry, read.brep_blob, read.face_colors);
        // Its features build again on the new base.
        registry.invalidate_body(document, body);
    } else {
        document.derived_mut().set_imported_geometry(body, geometry);
        document
            .derived_mut()
            .set_imported_brep_data(body, read.brep_blob, read.face_colors);
    }
    Landed::Shape
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel_api::{LimitToCome, SizeLimit};

    fn to_come(reading: u64) -> SolidBuildResult {
        SolidBuildResult {
            size_limit_to_come: Some(LimitToCome {
                op_index: 0,
                reading,
            }),
            ..SolidBuildResult::default()
        }
    }

    fn limit(largest: f64) -> SizeLimit {
        SizeLimit {
            largest: Some(largest),
            ..SizeLimit::default()
        }
    }

    /// A limit read after its build lands while the feature's last build
    /// awaits that reading: one a newer build of another history replaced,
    /// or one a newer build found itself, is dropped, and until a reading
    /// lands the feature has no limit to stop a drag at.
    #[test]
    fn a_limit_read_after_a_newer_build_is_dropped() {
        let mut document = Document::new("limits");
        let feature = FeatureId(uuid::Uuid::from_u128(1));
        let land = |document: &mut Document, result: &SolidBuildResult| {
            store_answers(document, &[feature], &[], &[], result);
        };
        let known = |document: &Document| document.derived().size_limit(feature).cloned();

        land(&mut document, &to_come(1));
        land(&mut document, &to_come(2));
        assert!(!store_read_limit(&mut document, feature, 1, limit(1.0)));
        assert_eq!(known(&document), None);
        assert!(store_read_limit(&mut document, feature, 2, limit(2.0)));
        assert_eq!(known(&document), Some(limit(2.0)));

        land(&mut document, &to_come(3));
        assert_eq!(known(&document), None, "a new reading voids the old");
        let found = SolidBuildResult {
            size_limit: Some(limit(4.0)),
            ..SolidBuildResult::default()
        };
        land(&mut document, &found);
        assert!(!store_read_limit(&mut document, feature, 3, limit(3.0)));
        assert_eq!(known(&document), Some(limit(4.0)));
    }
}
