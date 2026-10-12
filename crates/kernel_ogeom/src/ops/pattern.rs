//! Pattern re-application of tool solids under 4×4 transforms.
//!
//! Isometries (rotations, translations, mirrors) re-run the original tool op
//! with its world-space inputs (profile plane, placement) moved, so every
//! instance is concrete, in-place geometry; a mirrored primitive is built
//! where it is and mirrored whole. Scales and general affine maps go through
//! the kernel's NURBS rebuild (`general_transformed_shape`), which also
//! produces concrete topology.

use kernel_api::{
    ExtrudeTermination, PipeFrame, Placement, Profile, ProfilePlane, SolidOp, SweepKind,
};
use ogeom::algo::{copied, general_transformed_shape, transformed};
use ogeom::math::{GeneralTransform, Matrix3, Transform, Vector};
use ogeom::topo::{Model, Shape};

use super::tol;

pub enum PatternTool {
    /// Re-run this shape-producing op per instance.
    Op(Box<SolidOp>),
    /// Re-apply a solid as it stands: the whole running solid, or the
    /// tool of an op that reads the solid it was built on (a face swept
    /// off it), which a moved copy of the op would not find.
    Solid(Shape),
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
        for tool in tools {
            let placed = place(model, &acc, &tool.tool, matrix)?;
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

/// The copies of `solid` under each transform, fused, without the solid
/// itself.
pub fn copies_of(
    model: &mut Model,
    solid: &Shape,
    transforms: &[[[f64; 4]; 4]],
) -> Result<Shape, String> {
    let (first, rest) = transforms
        .split_first()
        .ok_or("every occurrence of the pattern is skipped: nothing is left of the body")?;
    let tool = PatternTool::Solid(solid.clone());
    let start = place(model, solid, &tool, first)?;
    apply(
        model,
        start,
        &[ToolInstance {
            tool,
            subtractive: false,
        }],
        rest,
    )
}

/// One tool placed by `matrix`, built against the running solid `acc`.
fn place(
    model: &mut Model,
    acc: &Shape,
    tool: &PatternTool,
    matrix: &[[f64; 4]; 4],
) -> Result<Shape, String> {
    let rigid = rigid_of(matrix);
    Ok(match (tool, &rigid) {
        // A primitive is placed by a frame a mirror would make
        // left-handed: it is built where it is and mirrored whole.
        (PatternTool::Op(op), None)
            if is_isometry(matrix) && matches!(**op, SolidOp::Primitive { .. }) =>
        {
            let tool = build_tool_op(model, Some(acc), op)?;
            transformed(model, &tool, reflection_of(matrix))
                .map_err(|e| format!("pattern mirror failed: {e}"))?
                .shape
        }
        // Isometries (rotations, translations, mirrors) re-run the
        // op with mapped inputs: exact, analytic, concrete.
        (PatternTool::Op(op), _) if is_isometry(matrix) => {
            let moved = transformed_op(op, matrix);
            build_tool_op(model, Some(acc), &moved)?
        }
        (PatternTool::Op(op), _) => {
            let base_tool = build_tool_op(model, Some(acc), op)?;
            general_transformed_shape(model, &base_tool, &general_of(matrix), tol())
                .map_err(|e| format!("pattern general transform failed: {e}"))?
                .shape
        }
        (PatternTool::Solid(shape), _) if is_isometry(matrix) => {
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
        (PatternTool::Solid(shape), _) => {
            general_transformed_shape(model, shape, &general_of(matrix), tol())
                .map_err(|e| format!("pattern general transform failed: {e}"))?
                .shape
        }
    })
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
        SolidOp::LoftThrough {
            sections,
            ruled,
            closed,
            ..
        } => super::loft_pipe::loft_through_tool(model, base, sections, *ruled, *closed),
        SolidOp::Pipe {
            profile,
            spine,
            frame,
            corner,
            sections,
            ..
        } => super::loft_pipe::pipe_tool(model, profile, spine, frame, *corner, sections),
        SolidOp::PipeThrough {
            profile,
            path,
            frame,
            corner,
            sections,
            ..
        } => super::loft_pipe::pipe_through_tool(
            model, base, profile, path, frame, *corner, sections,
        ),
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

/// Whether the matrix turns space inside out (a mirror).
fn reflects(m: &[[f64; 4]; 4]) -> bool {
    let c0 = Vector::new(m[0][0], m[1][0], m[2][0]);
    let c1 = Vector::new(m[0][1], m[1][1], m[2][1]);
    let c2 = Vector::new(m[0][2], m[1][2], m[2][2]);
    c0.dot(c1.cross(c2)) < 0.0
}

/// The profile moved by `m`. A mirror would leave its plane's frame
/// left-handed, which a sweep reads as a frame whose y axis points the
/// other way; the frame keeps its right hand by turning its y axis round,
/// and the wires follow by turning their v coordinates round, so every
/// point lands where the mirror puts it.
fn map_profile(m: &[[f64; 4]; 4], profile: &Profile) -> Profile {
    let mut plane = map_plane(m, &profile.plane);
    if !reflects(m) {
        return Profile {
            plane,
            wires: profile.wires.clone(),
        };
    }
    plane.y_axis = plane.y_axis.map(|v| -v);
    Profile {
        plane,
        wires: profile.wires.iter().map(flip_v_wire).collect(),
    }
}

fn flip_v(p: [f64; 2]) -> [f64; 2] {
    [p[0], -p[1]]
}

/// A wire with its v coordinates turned round.
fn flip_v_wire(wire: &kernel_api::ProfileWire) -> kernel_api::ProfileWire {
    use kernel_api::ProfileSegment as S;
    let segments = wire
        .segments
        .iter()
        .map(|segment| match segment {
            S::Line { start, end } => S::Line {
                start: flip_v(*start),
                end: flip_v(*end),
            },
            S::Arc { start, mid, end } => S::Arc {
                start: flip_v(*start),
                mid: flip_v(*mid),
                end: flip_v(*end),
            },
            S::Circle { center, radius } => S::Circle {
                center: flip_v(*center),
                radius: *radius,
            },
            S::Ellipse {
                center,
                major,
                ratio,
            } => S::Ellipse {
                center: flip_v(*center),
                major: flip_v(*major),
                ratio: *ratio,
            },
            // Mirrored, the arc runs the other way round its ellipse: the
            // point at parameter t lands at -t.
            S::EllipseArc {
                center,
                major,
                ratio,
                start_param,
                end_param,
            } => S::EllipseArc {
                center: flip_v(*center),
                major: flip_v(*major),
                ratio: *ratio,
                start_param: -end_param,
                end_param: -start_param,
            },
            S::BSpline {
                control_points,
                periodic,
            } => S::BSpline {
                control_points: control_points.iter().map(|p| flip_v(*p)).collect(),
                periodic: *periodic,
            },
            S::Nurbs {
                degree,
                knots,
                control_points,
                weights,
                periodic,
            } => S::Nurbs {
                degree: *degree,
                knots: knots.clone(),
                control_points: control_points.iter().map(|p| flip_v(*p)).collect(),
                weights: weights.clone(),
                periodic: *periodic,
            },
        })
        .collect();
    kernel_api::ProfileWire::new(segments)
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
                // The axis lies in the profile's plane: its v coordinates
                // turn round with the wires under a mirror, and a mirror
                // reverses the sense of turning, which turning about the
                // opposite direction gives back.
                SweepKind::Revolve {
                    axis_origin,
                    axis_dir,
                    angle_deg,
                    second_angle_deg,
                    midplane,
                    reversed,
                    termination,
                } if reflects(m) => SweepKind::Revolve {
                    axis_origin: flip_v(*axis_origin),
                    axis_dir: [-axis_dir[0], axis_dir[1]],
                    angle_deg: *angle_deg,
                    second_angle_deg: *second_angle_deg,
                    midplane: *midplane,
                    reversed: *reversed,
                    termination: *termination,
                },
                // A mirrored helix climbs the same axis, turning the other
                // way.
                SweepKind::Helix {
                    axis_origin,
                    axis_dir,
                    pitch,
                    height,
                    left_handed,
                    cone_angle_deg,
                    reversed,
                    turns,
                    growth,
                } if reflects(m) => SweepKind::Helix {
                    axis_origin: flip_v(*axis_origin),
                    axis_dir: flip_v(*axis_dir),
                    pitch: *pitch,
                    height: *height,
                    left_handed: !left_handed,
                    cone_angle_deg: *cone_angle_deg,
                    reversed: *reversed,
                    turns: *turns,
                    growth: *growth,
                },
                SweepKind::HelixNormal {
                    axis_origin,
                    pitch,
                    height,
                    left_handed,
                    cone_angle_deg,
                    reversed,
                    turns,
                    growth,
                } if reflects(m) => SweepKind::HelixNormal {
                    axis_origin: flip_v(*axis_origin),
                    pitch: *pitch,
                    height: *height,
                    left_handed: !left_handed,
                    cone_angle_deg: *cone_angle_deg,
                    reversed: *reversed,
                    turns: *turns,
                    growth: *growth,
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
        SolidOp::LoftThrough {
            sections,
            ruled,
            closed,
            op,
        } => SolidOp::LoftThrough {
            sections: sections.iter().map(|s| map_section(m, s)).collect(),
            ruled: *ruled,
            closed: *closed,
            op: *op,
        },
        SolidOp::Pipe {
            profile,
            spine,
            frame,
            corner,
            sections,
            op,
        } => SolidOp::Pipe {
            profile: map_profile(m, profile),
            spine: map_profile(m, spine),
            frame: match frame {
                PipeFrame::Auxiliary { path } => PipeFrame::Auxiliary {
                    path: map_profile(m, path),
                },
                PipeFrame::Binormal { direction } => PipeFrame::Binormal {
                    direction: map_vector(m, *direction),
                },
                other => other.clone(),
            },
            corner: *corner,
            sections: sections.iter().map(|s| map_profile(m, s)).collect(),
            op: *op,
        },
        // What lies on the solid is looked for where the transform puts
        // it, as the other ops' picks are.
        SolidOp::PipeThrough {
            profile,
            path,
            frame,
            corner,
            sections,
            op,
        } => SolidOp::PipeThrough {
            profile: map_section(m, profile),
            path: match path {
                kernel_api::PipePath::Profile(p) => {
                    kernel_api::PipePath::Profile(map_profile(m, p))
                }
                kernel_api::PipePath::Edges(edges) => {
                    kernel_api::PipePath::Edges(edges.iter().map(|e| map_edge(m, e)).collect())
                }
                kernel_api::PipePath::EdgesOf {
                    shape,
                    transform,
                    edges,
                } => kernel_api::PipePath::EdgesOf {
                    shape: shape.clone(),
                    transform: transform.clone(),
                    edges: edges.iter().map(|e| map_edge(m, e)).collect(),
                },
            },
            frame: match frame {
                PipeFrame::Auxiliary { path } => PipeFrame::Auxiliary {
                    path: map_profile(m, path),
                },
                PipeFrame::Binormal { direction } => PipeFrame::Binormal {
                    direction: map_vector(m, *direction),
                },
                other => other.clone(),
            },
            corner: *corner,
            sections: sections.iter().map(|s| map_section(m, s)).collect(),
            op: *op,
        },
        other => other.clone(),
    }
}

fn map_section(m: &[[f64; 4]; 4], section: &kernel_api::LoftSection) -> kernel_api::LoftSection {
    match section {
        kernel_api::LoftSection::Profile(p) => kernel_api::LoftSection::Profile(map_profile(m, p)),
        kernel_api::LoftSection::Face(probe) => {
            kernel_api::LoftSection::Face(kernel_api::FaceProbe {
                name: probe.name,
                point: map_point(m, probe.point),
                normal: map_vector(m, probe.normal),
            })
        }
        kernel_api::LoftSection::Point(p) => kernel_api::LoftSection::Point(map_point(m, *p)),
    }
}

fn map_edge(m: &[[f64; 4]; 4], edge: &kernel_api::EdgeProbe) -> kernel_api::EdgeProbe {
    kernel_api::EdgeProbe {
        point: map_point(m, edge.point),
        direction: map_vector(m, edge.direction),
        faces: edge.faces,
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
        other => other.clone(),
    }
}

/// Whether the matrix is an isometry (orthonormal, unit scale): rotations
/// and reflections. Both re-run the tool op exactly: a mirrored profile
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

/// Whether the matrix is a rotation or reflection to single precision:
/// what a placement kept in `f32` gives.
fn near_isometry(m: &[[f64; 4]; 4]) -> bool {
    const REACH: f64 = 1e-5;
    let c0 = Vector::new(m[0][0], m[1][0], m[2][0]);
    let c1 = Vector::new(m[0][1], m[1][1], m[2][1]);
    let c2 = Vector::new(m[0][2], m[1][2], m[2][2]);
    [c0, c1, c2]
        .iter()
        .all(|c| (c.magnitude() - 1.0).abs() < REACH)
        && c0.dot(c1).abs() < REACH
        && c1.dot(c2).abs() < REACH
        && c0.dot(c2).abs() < REACH
}

/// `shape` moved by a rigid row-major matrix, as a shape of its own.
pub(crate) fn moved(
    model: &mut Model,
    shape: &Shape,
    matrix: &[[f64; 4]; 4],
) -> Result<Shape, String> {
    let t = rigid_of(matrix).ok_or("a placement must be a rigid motion")?;
    Ok(transformed(model, shape, t)
        .map_err(|e| format!("moving the shape failed: {e}"))?
        .shape)
}

/// A copy of `shape` mirrored by the reflecting isometry `matrix`.
pub(crate) fn mirrored(
    model: &mut Model,
    shape: &Shape,
    matrix: &[[f64; 4]; 4],
) -> Result<Shape, String> {
    if !is_isometry(matrix) {
        return Err("a mirror must keep lengths".into());
    }
    let fresh = copied(model, shape)
        .map_err(|e| format!("copying the shape failed: {e}"))?
        .shape;
    Ok(transformed(model, &fresh, reflection_of(matrix))
        .map_err(|e| format!("mirroring the shape failed: {e}"))?
        .shape)
}

/// The similarity transform when the matrix is rigid (orthonormal, unit
/// scale, right-handed); `None` for reflections and the general path.
pub(crate) fn rigid_of(m: &[[f64; 4]; 4]) -> Option<Transform> {
    let c0 = Vector::new(m[0][0], m[1][0], m[2][0]);
    let c1 = Vector::new(m[0][1], m[1][1], m[2][1]);
    let c2 = Vector::new(m[0][2], m[1][2], m[2][2]);
    if !near_isometry(m) || c0.dot(c1.cross(c2)) < 0.0 {
        return None;
    }
    // A placement kept in single precision is a rotation to about 1e-7:
    // taken as the exact rotation its quaternion, made unit, is.
    let q = quaternion_from_columns(c0, c1, c2);
    let q = q.normalized(super::tol()).ok()?;
    let t = Vector::new(m[0][3], m[1][3], m[2][3]);
    Some(Transform::translation(t) * Transform::from_quaternion(q))
}

/// The similarity transform of a reflecting isometry: pull one plane mirror
/// out so the rest is a proper rotation, `L = R · mirror_x`.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A placement turned in single precision is a rigid motion: its
    /// columns are unit and square only to about 1e-7.
    #[test]
    fn a_single_precision_turn_is_rigid() {
        // A turn of 0.73 rad about (0.3, -0.5, 0.8), worked in f32.
        let n = (0.3f32 * 0.3 + 0.25 + 0.64).sqrt();
        let (x, y, z) = (0.3f32 / n, -0.5f32 / n, 0.8f32 / n);
        let (sin, cos) = 0.73f32.sin_cos();
        let t = 1.0 - cos;
        let r = [
            [t * x * x + cos, t * x * y - sin * z, t * x * z + sin * y],
            [t * x * y + sin * z, t * y * y + cos, t * y * z - sin * x],
            [t * x * z - sin * y, t * y * z + sin * x, t * z * z + cos],
        ];
        let shift = [4.0f32, -2.0, 7.5];
        let mut rows = [[0.0f64; 4]; 4];
        for i in 0..3 {
            for j in 0..3 {
                rows[i][j] = f64::from(r[i][j]);
            }
            rows[i][3] = f64::from(shift[i]);
        }
        rows[3][3] = 1.0;
        assert!(rigid_of(&rows).is_some());
        // A scaled one is not.
        rows[0][0] *= 1.01;
        assert!(rigid_of(&rows).is_none());
    }
}
