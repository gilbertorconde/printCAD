//! The features that shape a body as a whole: its base or a clone of
//! another body's solid, a split, a kept piece, a boolean with other
//! bodies, and lent geometry, which builds nothing.

use super::{sketch_profile, split_plane, tools_reach};
use core_document::{BodyId, BuildError, BuildPlan, Document};
use kernel_api::{BooleanOp, ExtrudeTermination, SolidOp, SweepKind};

use crate::feature::DesignFeature;

/// The ops of a feature that shapes the body as a whole, added to the plan.
pub(super) fn plan_body(
    document: &Document,
    body: BodyId,
    plan: &mut BuildPlan,
    feature: &DesignFeature,
    fail: &impl Fn(String) -> BuildError,
) -> Result<(), BuildError> {
    match feature {
        // Lent geometry builds nothing; the features that take it do.
        DesignFeature::Borrow { .. } => {}
        DesignFeature::Base {} => {
            if !plan.ops.is_empty() {
                return Err(fail(
                    "the base shape can only be a body's first feature".into(),
                ));
            }
            let brep = document
                .base_brep_blob(body)
                .ok_or_else(|| fail("this body has no base shape".into()))?
                .to_vec();
            plan.ops.push(SolidOp::Shape { brep });
        }
        DesignFeature::Clone { source } => {
            if !plan.ops.is_empty() {
                return Err(fail("a clone can only be a body's first feature".into()));
            }
            let brep = document
                .imported_brep_blob(*source)
                .ok_or_else(|| {
                    fail("the source body has no built solid yet (build it first)".into())
                })?
                .to_vec();
            plan.ops.push(SolidOp::Shape { brep });
        }
        DesignFeature::Split {
            plane,
            offset,
            sketch,
            other_side,
            split: _,
        } => match sketch {
            Some(sketch) => {
                let profile = sketch_profile(document, *sketch, &[]).map_err(&fail)?;
                let through = ExtrudeTermination::ThroughAll;
                plan.ops.push(SolidOp::Sweep {
                    profile,
                    kind: SweepKind::Extrude {
                        termination: through.clone(),
                        second_side: Some(through),
                        symmetric: false,
                        reversed: false,
                        taper_deg: 0.0,
                        direction: None,
                    },
                    op: if *other_side {
                        BooleanOp::Cut
                    } else {
                        BooleanOp::Common
                    },
                });
            }
            None => {
                let (origin, normal) = split_plane(document, plane, *offset).map_err(&fail)?;
                let normal = if *other_side {
                    normal.map(|v| -v)
                } else {
                    normal
                };
                plan.ops.push(SolidOp::HalfSpace { origin, normal });
            }
        },
        DesignFeature::Piece { near } => {
            plan.ops.push(SolidOp::KeepPiece {
                near: near.map(f64::from),
            });
        }
        DesignFeature::BodyBoolean {
            tool_body,
            kind,
            more_tools,
            refine: _,
            boolean_tolerance: _,
        } => {
            for tool_body in std::iter::once(tool_body).chain(more_tools) {
                if *tool_body == body {
                    return Err(fail(
                        "a body cannot be its own tool; pick another body".into(),
                    ));
                }
                if tools_reach(document, *tool_body, body) {
                    return Err(fail(
                        "the tool body takes this body as a tool in turn; one of the two \
                     has to go"
                            .into(),
                    ));
                }
                if document.body(*tool_body).is_none() {
                    return Err(fail(
                        "the tool body is not in this document; pick another body".into(),
                    ));
                }
                let tool_brep = document
                    .imported_brep_blob(*tool_body)
                    .ok_or_else(|| {
                        fail("the tool body has no built solid yet (build it first)".into())
                    })?
                    .to_vec();
                // The tool's shape is in its own body's frame; it meets this
                // body's where the two bodies sit.
                let relative = document
                    .body_placement(body)
                    .inverse()
                    .after(&document.body_placement(*tool_body));
                plan.ops.push(SolidOp::Boolean {
                    tool_brep,
                    kind: *kind,
                    tool_transform: (!relative.is_identity()).then(|| relative.rows()),
                });
            }
        }
        _ => {}
    }
    Ok(())
}
