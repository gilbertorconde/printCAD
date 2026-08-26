//! Pattern re-application of tool solids under 4×4 transforms.
//!
//! Rigid transforms re-run the original tool op with its world-space inputs
//! (profile plane, placement) moved — every instance is concrete, in-place
//! geometry, the same effect the previous kernel got from re-loading a
//! serialized tool blob under a rigid transform. Mirrors, scales and general
//! affine maps go through the kernel's NURBS rebuild
//! (`general_transformed_shape`), which also produces concrete topology.

use kernel_api::{ExtrudeTermination, Placement, Profile, ProfilePlane, SolidOp, SweepKind};
use ogeom::algo::{copied, general_transformed_shape, transformed};
use ogeom::math::{GeneralTransform, Matrix3, Transform, Vector};
use ogeom::topo::{Model, Shape};

use super::tol;

pub enum PatternTool {
    /// Re-run this shape-producing op per instance.
    Op(Box<SolidOp>),
    /// Re-apply the whole running solid.
    WholeBody(Shape),
}

pub struct ToolInstance {
    pub tool: PatternTool,
    pub subtractive: bool,
}

/// Apply each transform to each tool, fusing additive tools and cutting
/// subtractive ones into the running solid.
pub fn apply(
    model: &mut Model,
    current: Shape,
    tools: &[ToolInstance],
    transforms: &[[[f64; 4]; 4]],
) -> Result<Shape, String> {
    let mut acc = current;
    for matrix in transforms {
        let rigid = rigid_of(matrix);
        for tool in tools {
            let placed = match (&tool.tool, &rigid) {
                // Isometries (rotations, translations, mirrors) re-run the
                // op with mapped inputs — exact, analytic, concrete.
                (PatternTool::Op(op), _) if is_isometry(matrix) => {
                    let moved = transformed_op(op, matrix);
                    build_tool_op(model, Some(&acc), &moved)?
                }
                (PatternTool::Op(op), _) => {
                    let base_tool = build_tool_op(model, Some(&acc), op)?;
                    general_transformed_shape(model, &base_tool, &general_of(matrix), tol())
                        .map_err(|e| format!("pattern general transform failed: {e}"))?
                        .shape
                }
                (PatternTool::WholeBody(shape), _) if is_isometry(matrix) => {
                    // A fresh copy: the boolean must see an independent
                    // operand, not the running solid under a placement.
                    let fresh = copied(model, shape)
                        .map_err(|e| format!("pattern copy failed: {e}"))?
                        .shape;
                    let t = rigid.unwrap_or_else(|| reflection_of(matrix));
                    transformed(model, &fresh, t)
                        .map_err(|e| format!("pattern transform failed: {e}"))?
                        .shape
                }
                (PatternTool::WholeBody(shape), _) => {
                    general_transformed_shape(model, shape, &general_of(matrix), tol())
                        .map_err(|e| format!("pattern general transform failed: {e}"))?
                        .shape
                }
            };
            let kind = if tool.subtractive {
                kernel_api::BoolKind::Cut
            } else {
                kernel_api::BoolKind::Fuse
            };
            acc = super::combine_solids(model, &acc, &placed, kind)
                .map_err(|e| format!("pattern boolean failed: {e}"))?;
        }
    }
    Ok(acc)
}

fn build_tool_op(model: &mut Model, base: Option<&Shape>, op: &SolidOp) -> Result<Shape, String> {
    match op {
        SolidOp::Sweep { profile, kind, .. } => {
            super::sweep::build_tool(model, base, profile, kind)
        }
        SolidOp::Primitive {
            kind, placement, ..
        } => super::primitive::build_tool(model, kind, placement),
        SolidOp::Loft {
            sections,
            ruled,
            closed,
            ..
        } => super::loft_pipe::loft_tool(model, sections, *ruled, *closed),
        SolidOp::Pipe {
            profile,
            spine,
            frenet,
            ..
        } => super::loft_pipe::pipe_tool(model, profile, spine, *frenet),
        _ => Err("pattern references an op that produces no tool solid".into()),
    }
}

fn map_point(m: &[[f64; 4]; 4], p: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2] + m[0][3],
        m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2] + m[1][3],
        m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2] + m[2][3],
    ]
}

fn map_vector(m: &[[f64; 4]; 4], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn map_plane(m: &[[f64; 4]; 4], plane: &ProfilePlane) -> ProfilePlane {
    ProfilePlane {
        origin: map_point(m, plane.origin),
        x_axis: map_vector(m, plane.x_axis),
        y_axis: map_vector(m, plane.y_axis),
        normal: map_vector(m, plane.normal),
    }
}

fn map_profile(m: &[[f64; 4]; 4], profile: &Profile) -> Profile {
    Profile {
        plane: map_plane(m, &profile.plane),
        wires: profile.wires.clone(),
    }
}

/// A shape-producing op with its world-space inputs moved by a rigid
/// transform. The 2D payloads (wires, in-plane axes) ride along with their
/// plane unchanged.
fn transformed_op(op: &SolidOp, m: &[[f64; 4]; 4]) -> SolidOp {
    match op {
        SolidOp::Sweep { profile, kind, op } => SolidOp::Sweep {
            profile: map_profile(m, profile),
            kind: match kind {
                SweepKind::Extrude {
                    termination,
                    second_side,
                    symmetric,
                    reversed,
                    taper_deg,
                    direction,
                } => SweepKind::Extrude {
                    termination: map_termination(m, termination),
                    second_side: second_side.as_ref().map(|t| map_termination(m, t)),
                    symmetric: *symmetric,
                    reversed: *reversed,
                    taper_deg: *taper_deg,
                    direction: direction.map(|d| map_vector(m, d)),
                },
                other => other.clone(),
            },
            op: *op,
        },
        SolidOp::Primitive {
            kind,
            placement,
            op,
        } => SolidOp::Primitive {
            kind: *kind,
            placement: Placement {
                origin: map_point(m, placement.origin),
                x_axis: map_vector(m, placement.x_axis),
                z_axis: map_vector(m, placement.z_axis),
            },
            op: *op,
        },
        SolidOp::Loft {
            sections,
            ruled,
            closed,
            op,
        } => SolidOp::Loft {
            sections: sections.iter().map(|s| map_profile(m, s)).collect(),
            ruled: *ruled,
            closed: *closed,
            op: *op,
        },
        SolidOp::Pipe {
            profile,
            spine,
            frenet,
            op,
        } => SolidOp::Pipe {
            profile: map_profile(m, profile),
            spine: map_profile(m, spine),
            frenet: *frenet,
            op: *op,
        },
        other => other.clone(),
    }
}

fn map_termination(m: &[[f64; 4]; 4], term: &ExtrudeTermination) -> ExtrudeTermination {
    match term {
        ExtrudeTermination::UpToPlane {
            point,
            normal,
            offset,
        } => ExtrudeTermination::UpToPlane {
            point: map_point(m, *point),
            normal: map_vector(m, *normal),
            offset: *offset,
        },
        other => *other,
    }
}

/// Whether the matrix is an isometry (orthonormal, unit scale) — rotations
/// AND reflections. Both re-run the tool op exactly: a mirrored profile
/// plane produces the mirrored solid, since the 2D payloads map through the
/// mirrored axes. Only scaling/shear routes to the general (NURBS) path.
fn is_isometry(m: &[[f64; 4]; 4]) -> bool {
    let c0 = Vector::new(m[0][0], m[1][0], m[2][0]);
    let c1 = Vector::new(m[0][1], m[1][1], m[2][1]);
    let c2 = Vector::new(m[0][2], m[1][2], m[2][2]);
    let unit = (c0.magnitude() - 1.0).abs() < 1e-9
        && (c1.magnitude() - 1.0).abs() < 1e-9
        && (c2.magnitude() - 1.0).abs() < 1e-9;
    let ortho = c0.dot(c1).abs() < 1e-9 && c1.dot(c2).abs() < 1e-9 && c0.dot(c2).abs() < 1e-9;
    unit && ortho
}

/// The similarity transform when the matrix is rigid (orthonormal, unit
/// scale, right-handed); `None` for reflections and the general path.
fn rigid_of(m: &[[f64; 4]; 4]) -> Option<Transform> {
    let c0 = Vector::new(m[0][0], m[1][0], m[2][0]);
    let c1 = Vector::new(m[0][1], m[1][1], m[2][1]);
    let c2 = Vector::new(m[0][2], m[1][2], m[2][2]);
    if !is_isometry(m) || c0.dot(c1.cross(c2)) < 0.0 {
        return None;
    }
    let q = quaternion_from_columns(c0, c1, c2);
    let t = Vector::new(m[0][3], m[1][3], m[2][3]);
    Some(Transform::translation(t) * Transform::from_quaternion(q))
}

/// The similarity transform of a reflecting isometry: pull one plane mirror
/// out so the rest is a proper rotation — `L = R · mirror_x`.
fn reflection_of(m: &[[f64; 4]; 4]) -> Transform {
    let c0 = Vector::new(m[0][0], m[1][0], m[2][0]);
    let c1 = Vector::new(m[0][1], m[1][1], m[2][1]);
    let c2 = Vector::new(m[0][2], m[1][2], m[2][2]);
    let q = quaternion_from_columns(-c0, c1, c2);
    let t = Vector::new(m[0][3], m[1][3], m[2][3]);
    Transform::translation(t)
        * Transform::from_quaternion(q)
        * Transform::plane_mirror(
            ogeom::math::Point::new(0.0, 0.0, 0.0),
            ogeom::math::Direction::X,
        )
}

fn general_of(m: &[[f64; 4]; 4]) -> GeneralTransform {
    GeneralTransform {
        linear: Matrix3::from_columns(
            Vector::new(m[0][0], m[1][0], m[2][0]),
            Vector::new(m[0][1], m[1][1], m[2][1]),
            Vector::new(m[0][2], m[1][2], m[2][2]),
        ),
        translation: Vector::new(m[0][3], m[1][3], m[2][3]),
    }
}

/// Shepperd's method over a proper rotation given by its columns.
fn quaternion_from_columns(c0: Vector, c1: Vector, c2: Vector) -> ogeom::math::Quaternion {
    let (m00, m01, m02) = (c0.x, c1.x, c2.x);
    let (m10, m11, m12) = (c0.y, c1.y, c2.y);
    let (m20, m21, m22) = (c0.z, c1.z, c2.z);
    let trace = m00 + m11 + m22;
    let (w, x, y, z) = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        (s * 0.25, (m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s)
    } else if m00 > m11 && m00 > m22 {
        let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
        ((m21 - m12) / s, s * 0.25, (m01 + m10) / s, (m02 + m20) / s)
    } else if m11 > m22 {
        let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
        ((m02 - m20) / s, (m01 + m10) / s, s * 0.25, (m12 + m21) / s)
    } else {
        let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
        ((m10 - m01) / s, (m02 + m20) / s, (m12 + m21) / s, s * 0.25)
    };
    ogeom::math::Quaternion::new(w, x, y, z)
}
