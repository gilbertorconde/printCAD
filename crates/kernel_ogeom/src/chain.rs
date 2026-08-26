//! `execute_solid_chain`: threads one in-memory `(Model, Shape)` through a
//! body's `SolidOp` list. Native-format blobs appear only at the boundaries:
//! the final result out, and `SolidOp::Boolean`'s external tool in. Tool
//! snapshots for patterns are in-model `Shape`s — no per-op serialization.

use kernel_api::{
    BoolKind, BooleanOp, ChainError, SolidBuildResult, SolidOp, TessellationSettings,
};
use ogeom::topo::{Model, Shape};

use crate::ops::{self, pattern};
use crate::{progress, tess};

struct ToolSnapshot {
    op: SolidOp,
    subtractive: bool,
}

pub fn execute(
    ops_list: &[SolidOp],
    detail: &TessellationSettings,
) -> Result<SolidBuildResult, ChainError> {
    let chain_err = |op_index: usize, message: String| ChainError { op_index, message };

    if ops_list.is_empty() {
        return Err(chain_err(0, "solid-op chain is empty".into()));
    }
    match ops_list[0].boolean_op() {
        Some(BooleanOp::NewSolid) => {}
        Some(_) => {
            return Err(chain_err(
                0,
                "first solid op in a chain must be NewSolid".into(),
            ))
        }
        None => {
            return Err(chain_err(
                0,
                "first solid op in a chain must produce a shape".into(),
            ))
        }
    }
    for (index, op) in ops_list.iter().enumerate().skip(1) {
        if op.boolean_op() == Some(BooleanOp::NewSolid) {
            return Err(chain_err(
                index,
                "only the first op in a chain may be NewSolid".into(),
            ));
        }
    }

    let mut model = Model::with_tolerances(tess::tolerances());
    let mut current: Option<Shape> = None;
    let mut tools: Vec<Option<ToolSnapshot>> = Vec::with_capacity(ops_list.len());

    for (index, solid_op) in ops_list.iter().enumerate() {
        progress::context(format_args!(
            "{} {}/{}",
            progress::op_label(solid_op),
            index + 1,
            ops_list.len()
        ));
        let err = |message: String| ChainError {
            op_index: index,
            message,
        };
        progress::checkpoint().map_err(&err)?;
        let base = current.clone();
        let mut tool_snapshot: Option<ToolSnapshot> = None;

        let next = match solid_op {
            SolidOp::Sweep { profile, kind, op } => {
                let tool = ops::sweep::build_tool(&mut model, base.as_ref(), profile, kind)
                    .map_err(&err)?;
                tool_snapshot = Some(ToolSnapshot {
                    op: solid_op.clone(),
                    subtractive: *op == BooleanOp::Cut,
                });
                combine(&mut model, base.as_ref(), tool, *op).map_err(&err)?
            }
            SolidOp::Primitive {
                kind,
                placement,
                op,
            } => {
                let tool = ops::primitive::build_tool(&mut model, kind, placement).map_err(&err)?;
                tool_snapshot = Some(ToolSnapshot {
                    op: solid_op.clone(),
                    subtractive: *op == BooleanOp::Cut,
                });
                combine(&mut model, base.as_ref(), tool, *op).map_err(&err)?
            }
            SolidOp::Loft {
                sections,
                ruled,
                closed,
                op,
            } => {
                let tool = ops::loft_pipe::loft_tool(&mut model, sections, *ruled, *closed)
                    .map_err(&err)?;
                tool_snapshot = Some(ToolSnapshot {
                    op: solid_op.clone(),
                    subtractive: *op == BooleanOp::Cut,
                });
                combine(&mut model, base.as_ref(), tool, *op).map_err(&err)?
            }
            SolidOp::Pipe {
                profile,
                spine,
                frenet,
                op,
            } => {
                let tool =
                    ops::loft_pipe::pipe_tool(&mut model, profile, spine, *frenet).map_err(&err)?;
                tool_snapshot = Some(ToolSnapshot {
                    op: solid_op.clone(),
                    subtractive: *op == BooleanOp::Cut,
                });
                combine(&mut model, base.as_ref(), tool, *op).map_err(&err)?
            }
            SolidOp::Fillet { radius, edges } => {
                let solid = base.ok_or_else(|| err("fillet needs an existing solid".into()))?;
                ops::dressup::fillet(&mut model, &solid, *radius, edges).map_err(&err)?
            }
            SolidOp::Chamfer { spec, flip, edges } => {
                let solid = base.ok_or_else(|| err("chamfer needs an existing solid".into()))?;
                ops::dressup::chamfer(&mut model, &solid, spec, *flip, edges).map_err(&err)?
            }
            SolidOp::Draft {
                angle_deg,
                neutral_point,
                neutral_normal,
                pull_dir,
                faces,
            } => {
                let solid = base.ok_or_else(|| err("draft needs an existing solid".into()))?;
                ops::dressup::draft(
                    &mut model,
                    &solid,
                    *angle_deg,
                    *neutral_point,
                    *neutral_normal,
                    *pull_dir,
                    faces,
                )
                .map_err(&err)?
            }
            SolidOp::Thickness {
                value,
                open_faces,
                inward,
            } => {
                let solid = base.ok_or_else(|| err("thickness needs an existing solid".into()))?;
                ops::dressup::thickness(&mut model, &solid, *value, open_faces, *inward)
                    .map_err(&err)?
            }
            SolidOp::Transform {
                transforms,
                originals,
            } => {
                let solid = base.ok_or_else(|| err("pattern needs an existing solid".into()))?;
                let instances: Vec<pattern::ToolInstance> = if originals.is_empty() {
                    vec![pattern::ToolInstance {
                        tool: pattern::PatternTool::WholeBody(solid.clone()),
                        subtractive: false,
                    }]
                } else {
                    originals
                        .iter()
                        .map(|&orig| {
                            tools
                                .get(orig)
                                .and_then(|t| t.as_ref())
                                .map(|t| pattern::ToolInstance {
                                    tool: pattern::PatternTool::Op(Box::new(t.op.clone())),
                                    subtractive: t.subtractive,
                                })
                                .ok_or_else(|| {
                                    err("pattern references an op with no reusable tool solid"
                                        .into())
                                })
                        })
                        .collect::<Result<_, _>>()?
                };
                pattern::apply(&mut model, solid, &instances, transforms).map_err(&err)?
            }
            SolidOp::Boolean { tool_brep, kind } => {
                let solid = base.ok_or_else(|| err("boolean needs an existing solid".into()))?;
                external_boolean(&mut model, &solid, tool_brep, *kind).map_err(&err)?
            }
        };

        current = Some(next);
        tools.push(tool_snapshot);
    }

    let final_shape = current.expect("chain validated non-empty");
    let mesh = tess::mesh_shape(&model, &final_shape, &[], detail).map_err(|e| {
        chain_err(
            ops_list.len() - 1,
            format!("meshing the result failed: {e}"),
        )
    })?;
    if mesh.positions.is_empty() || mesh.indices.is_empty() {
        return Err(chain_err(
            ops_list.len() - 1,
            "solid-op chain produced an empty render mesh".into(),
        ));
    }
    let brep_blob = tess::write_blob(&model, &final_shape).map_err(|e| {
        chain_err(
            ops_list.len() - 1,
            format!("serializing the result failed: {e}"),
        )
    })?;

    let bounds_mm = mesh.bounds();
    Ok(SolidBuildResult {
        brep_blob,
        mesh,
        bounds_mm,
    })
}

/// Combine a freshly built tool with the running solid.
fn combine(
    model: &mut Model,
    base: Option<&Shape>,
    tool: Shape,
    op: BooleanOp,
) -> Result<Shape, String> {
    match op {
        BooleanOp::NewSolid => Ok(tool),
        BooleanOp::Fuse => {
            let base =
                base.ok_or_else(|| "fuse requires existing material in the body".to_string())?;
            if !ops::bounds_overlap(model, base, &tool) {
                return ops::fuse_or_compound(model, base, &tool);
            }
            ops::combine_solids(model, base, &tool, BoolKind::Fuse)
        }
        BooleanOp::Cut => {
            let base =
                base.ok_or_else(|| "cut requires existing material in the body".to_string())?;
            ops::combine_solids(model, base, &tool, BoolKind::Cut)
        }
    }
}

/// Boolean against an external body's serialized snapshot.
fn external_boolean(
    model: &mut Model,
    solid: &Shape,
    tool_brep: &[u8],
    kind: BoolKind,
) -> Result<Shape, String> {
    let text = std::str::from_utf8(tool_brep)
        .map_err(|_| "boolean tool snapshot is not valid UTF-8".to_string())?;
    let absorbed = ogeom::io::native::read_into(model, text)
        .map_err(|e| format!("importing the boolean tool solid failed: {e}"))?;
    let tool = absorbed
        .shapes
        .first()
        .ok_or_else(|| "boolean tool snapshot holds no shape".to_string())?
        .clone();
    ops::combine_solids(model, solid, &tool, kind)
}
