//! Loft through section profiles and sweep along a sketch spine.

use kernel_api::Profile;
use ogeom::algo::transformed;
use ogeom::geom::Curve3d;
use ogeom::math::{Axis, Direction, Point, Transform, Vector};
use ogeom::offset::{make_loft, make_loft_skinned, make_loft_skinned_closed, make_pipe_shell};
use ogeom::topo::{Model, NodeData, Shape};

use super::{fuse_all, tol};
use crate::profile::{self, BuiltProfile};

const SKIN_TOLERANCE: f64 = 1e-3;

struct Section {
    outer: Shape,
    holes: Vec<Shape>,
}

fn single_region(model: &mut Model, prof: &Profile, what: &str) -> Result<Section, String> {
    let built = profile::build_profile(model, prof)?;
    let mut groups = built.groups;
    if groups.len() != 1 {
        return Err(format!("{what} must enclose a single region"));
    }
    let group = groups.remove(0);
    Ok(Section {
        outer: group.outer.wire,
        holes: group.holes.into_iter().map(|h| h.wire).collect(),
    })
}

pub fn loft_tool(
    model: &mut Model,
    sections: &[Profile],
    ruled: bool,
    closed: bool,
) -> Result<Shape, String> {
    if sections.len() < 2 {
        return Err("loft needs at least two sections".into());
    }
    let built: Vec<Section> = sections
        .iter()
        .map(|p| single_region(model, p, "a loft section"))
        .collect::<Result<_, _>>()?;

    let hole_count = built[0].holes.len();
    if built.iter().any(|s| s.holes.len() != hole_count) {
        return Err("loft sections must have matching hole counts".into());
    }

    let outers: Vec<Shape> = built.iter().map(|s| s.outer.clone()).collect();
    let mut solid = loft_wires(model, &outers, ruled, closed)?;

    for hole in 0..hole_count {
        let hole_wires: Vec<Shape> = built.iter().map(|s| s.holes[hole].clone()).collect();
        let hole_solid = loft_wires(model, &hole_wires, ruled, closed)?;
        solid = ogeom::boolean::cut(model, &solid, &hole_solid, tol())
            .map_err(|e| format!("subtracting a hole loft failed: {e}"))?
            .shape;
    }
    Ok(solid)
}

fn loft_wires(
    model: &mut Model,
    wires: &[Shape],
    ruled: bool,
    closed: bool,
) -> Result<Shape, String> {
    if closed {
        return make_loft_skinned_closed(model, wires, SKIN_TOLERANCE, tol())
            .map(|b| b.shape)
            .map_err(|e| format!("closed loft failed: {e}"));
    }
    if ruled {
        // Chain of two-section ruled lofts, fused.
        let mut parts = Vec::with_capacity(wires.len() - 1);
        for pair in wires.windows(2) {
            let part = make_loft(model, &pair[0], &pair[1], tol())
                .map_err(|e| format!("ruled loft failed: {e}"))?;
            parts.push(part.shape);
        }
        fuse_all(model, parts)
    } else {
        make_loft_skinned(model, wires, SKIN_TOLERANCE, tol())
            .map(|b| b.shape)
            .map_err(|e| format!("loft failed: {e}"))
    }
}

pub fn pipe_tool(
    model: &mut Model,
    prof: &Profile,
    spine: &Profile,
    frenet: bool,
) -> Result<Shape, String> {
    let built = profile::build_profile(model, prof)?;

    let spine_wire_desc = spine
        .wires
        .first()
        .ok_or_else(|| "pipe spine has no wire".to_string())?;
    let spine_wire = profile::build_wire(model, &spine.plane, spine_wire_desc, false)
        .map_err(|e| format!("pipe spine: {e}"))?;

    let (start, tangent) = spine_start(model, &spine_wire)?;

    // The sweep wants the profile sitting at the spine start, square to its
    // tangent. Sketches rarely oblige exactly: rotate the profile's normal
    // onto the tangent and move its centroid onto the start.
    let centroid = profile::profile_centroid(model, &built)?;
    let normal = profile::plane_normal(&prof.plane).map_err(|e| format!("pipe profile: {e}"))?;
    let rotate = rotation_aligning(normal, tangent, centroid);
    let translate = Transform::translation(start - centroid);
    let place = translate * rotate;

    let mut parts = Vec::with_capacity(built.faces.len());
    for face in &built.faces {
        let placed = transformed(model, face, place)
            .map_err(|e| format!("placing the pipe profile failed: {e}"))?
            .shape;
        let part = make_pipe_shell(model, &placed, &spine_wire, frenet, SKIN_TOLERANCE, tol())
            .map_err(|e| format!("pipe sweep failed: {e}"))?;
        parts.push(part.shape);
    }
    let _ = &built as &BuiltProfile;
    fuse_all(model, parts)
}

/// Start point and unit tangent of a wire's first edge.
fn spine_start(model: &Model, wire: &Shape) -> Result<(Point, Vector), String> {
    let edges = model
        .children_of(wire)
        .map_err(|e| format!("pipe spine wire: {e}"))?;
    let first = edges
        .first()
        .ok_or_else(|| "pipe spine wire has no edges".to_string())?;
    let node = model
        .node(first)
        .ok_or_else(|| "pipe spine edge is not in the model".to_string())?;
    let NodeData::Edge(data) = node.data() else {
        return Err("pipe spine child is not an edge".into());
    };
    for repr in &data.representations {
        if let ogeom::topo::EdgeRepr::Curve3d {
            curve,
            location,
            range,
        } = repr
        {
            let Some(geometry) = model.geometry().curve(*curve) else {
                continue;
            };
            let t0 = range.0;
            let dt = ((range.1 - range.0) * 1e-4).max(1e-9);
            let p0 = geometry
                .point_at(t0, tol())
                .map_err(|e| format!("pipe spine start: {e}"))?;
            let p1 = geometry
                .point_at(t0 + dt, tol())
                .map_err(|e| format!("pipe spine tangent: {e}"))?;
            let placement = location
                .composed(model.datums())
                .map_err(|e| format!("pipe spine placement: {e}"))?;
            let start = placement.apply(p0);
            let toward = placement.apply(p1) - start;
            let len = toward.magnitude();
            if len <= 1e-12 {
                return Err("pipe spine tangent is degenerate".into());
            }
            return Ok((start, toward * (1.0 / len)));
        }
    }
    Err("pipe spine edge carries no 3D curve".into())
}

/// Rotation about the profile centroid taking `normal` onto whichever of
/// ±`tangent` it is closer to. Identity when already aligned.
fn rotation_aligning(normal: Direction, tangent: Vector, about: Point) -> Transform {
    let n = normal.vector();
    let t = if n.dot(tangent) >= 0.0 {
        tangent
    } else {
        -tangent
    };
    let cross = n.cross(t);
    let sin = cross.magnitude();
    let cos = n.dot(t);
    let angle = sin.atan2(cos);
    if angle.abs() < 1e-9 {
        return Transform::IDENTITY;
    }
    let Ok(axis_dir) = Direction::new(cross, tol()) else {
        return Transform::IDENTITY;
    };
    Transform::rotation(Axis::new(about, axis_dir), angle)
}
