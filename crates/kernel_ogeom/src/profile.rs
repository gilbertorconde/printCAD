//! `kernel_api::Profile` → ogeom wires and faces.
//!
//! Mirrors the previous kernel's profile pipeline: segments become edges on
//! exact curves, wires group by containment (even nesting depth = outer
//! boundary, odd = hole of its immediate container), and each group becomes
//! one planar face — a multi-region sketch yields several faces.

use kernel_api::{Profile, ProfilePlane, ProfileSegment, ProfileWire};
use ogeom::algo::{
    classify_on_face, make_edge, make_edge_between, make_face_with_pcurves, make_wire,
    surface_properties,
};
use ogeom::core::OgeomResult;
use ogeom::geom::{
    BSplineCurve, CircleCurve, Curve, Curve3d, EllipseCurve, LineCurve, PlaneSurface,
    SurfaceGeometry,
};
use ogeom::math::{Circle, Direction, Ellipse, Frame, KnotVector, Plane, Point, Vector};
use ogeom::mesh::Deflection;
use ogeom::topo::{Model, Shape, VertexData};

use crate::tess;

const TAU: f64 = std::f64::consts::TAU;

/// A profile lifted into the model: one face per outer wire, with the wires
/// kept around for taper lofts and spine reuse.
pub struct BuiltProfile {
    /// One planar face per region (outer wire + its holes).
    pub faces: Vec<Shape>,
    /// The wire groups behind those faces, same order.
    pub groups: Vec<WireGroup>,
}

pub struct WireGroup {
    pub outer: BuiltWire,
    pub holes: Vec<BuiltWire>,
    /// Indices into the source profile's wire list (outer first).
    pub wire_indices: Vec<usize>,
}

/// A wire plus the edges it was assembled from (face construction attaches
/// pcurves per edge, so the edges stay useful after the wire exists).
#[derive(Clone)]
pub struct BuiltWire {
    pub wire: Shape,
    pub edges: Vec<Shape>,
}

pub fn world_point(plane: &ProfilePlane, u: f64, v: f64) -> Point {
    Point::new(
        plane.origin[0] + u * plane.x_axis[0] + v * plane.y_axis[0],
        plane.origin[1] + u * plane.x_axis[1] + v * plane.y_axis[1],
        plane.origin[2] + u * plane.x_axis[2] + v * plane.y_axis[2],
    )
}

pub fn world_vector(plane: &ProfilePlane, u: f64, v: f64) -> Vector {
    Vector::new(
        u * plane.x_axis[0] + v * plane.y_axis[0],
        u * plane.x_axis[1] + v * plane.y_axis[1],
        u * plane.x_axis[2] + v * plane.y_axis[2],
    )
}

pub fn plane_normal(plane: &ProfilePlane) -> OgeomResult<Direction> {
    Direction::new(
        Vector::new(plane.normal[0], plane.normal[1], plane.normal[2]),
        tess::tolerances(),
    )
}

/// The ogeom `Plane` (frame z = sketch normal, x = sketch x-axis).
pub fn plane_of(plane: &ProfilePlane) -> OgeomResult<Plane> {
    let tol = tess::tolerances();
    let origin = Point::new(plane.origin[0], plane.origin[1], plane.origin[2]);
    let z = plane_normal(plane)?;
    let x = Direction::new(
        Vector::new(plane.x_axis[0], plane.x_axis[1], plane.x_axis[2]),
        tol,
    )?;
    Ok(Plane::new(Frame::new(origin, z, x, tol)?))
}

/// A `PlaneSurface` just wide enough to hold the given wires. Kept tight on
/// purpose: a face's carrier bound feeds bounding-box queries, and a carrier
/// spanning the whole sketch would make disjoint regions look overlapping.
fn plane_surface(plane: &Plane, wires: &[ProfileWire]) -> OgeomResult<PlaneSurface> {
    let (umin, umax, vmin, vmax) = uv_bounds(wires);
    let margin = ((umax - umin) + (vmax - vmin)).max(1.0) * 0.05 + 0.5;
    PlaneSurface::over(
        *plane,
        (umin - margin, umax + margin),
        (vmin - margin, vmax + margin),
    )
}

/// Conservative 2D bounds over every coordinate a wire's segments mention.
fn uv_bounds(wires: &[ProfileWire]) -> (f64, f64, f64, f64) {
    let mut b = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    let mut push = |p: [f64; 2], r: f64| {
        b.0 = b.0.min(p[0] - r);
        b.1 = b.1.max(p[0] + r);
        b.2 = b.2.min(p[1] - r);
        b.3 = b.3.max(p[1] + r);
    };
    for wire in wires {
        for seg in &wire.segments {
            match seg {
                ProfileSegment::Line { start, end } => {
                    push(*start, 0.0);
                    push(*end, 0.0);
                }
                ProfileSegment::Arc { start, mid, end } => {
                    // The bulge never exceeds the circumcircle of the three
                    // points; padding by the max pairwise distance is enough.
                    let d = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
                    let pad = d(*start, *end).max(d(*start, *mid)).max(d(*mid, *end));
                    push(*start, pad);
                    push(*mid, pad);
                    push(*end, pad);
                }
                ProfileSegment::Circle { center, radius } => push(*center, *radius),
                ProfileSegment::Ellipse {
                    center,
                    major,
                    ratio: _,
                }
                | ProfileSegment::EllipseArc {
                    center,
                    major,
                    ratio: _,
                    ..
                } => {
                    push(*center, major[0].hypot(major[1]));
                }
                ProfileSegment::BSpline { control_points, .. } => {
                    for p in control_points {
                        push(*p, 0.0);
                    }
                }
            }
        }
    }
    if b.0 > b.1 {
        (0.0, 1.0, 0.0, 1.0)
    } else {
        b
    }
}

fn seg_err(what: &str) -> String {
    format!("profile segment: {what}")
}

/// The curve and parameter range for one segment, directed start → end.
fn segment_curve(
    plane: &ProfilePlane,
    normal: Direction,
    x_axis: Direction,
    seg: &ProfileSegment,
) -> Result<(Curve, (f64, f64)), String> {
    let tol = tess::tolerances();
    match seg {
        ProfileSegment::Line { start, end } => {
            let p1 = world_point(plane, start[0], start[1]);
            let p2 = world_point(plane, end[0], end[1]);
            let curve =
                LineCurve::segment(p1, p2, tol).map_err(|e| seg_err(&format!("line: {e}")))?;
            let range = curve.domain();
            Ok((Curve::Line(curve), range))
        }
        ProfileSegment::Arc { start, mid, end } => {
            let p1 = world_point(plane, start[0], start[1]);
            let pm = world_point(plane, mid[0], mid[1]);
            let p2 = world_point(plane, end[0], end[1]);
            let circle =
                Circle::through(p1, pm, p2, tol).map_err(|e| seg_err(&format!("arc: {e}")))?;
            let center = circle.frame().origin();
            let radius = circle.radius();
            // Re-frame the circle so the arc runs counter-clockwise from the
            // start point: z = ±plane normal picked by which way the arc
            // actually turns (mid must come before end going ccw).
            let x_dir = Direction::new(p1 - center, tol)
                .map_err(|e| seg_err(&format!("arc frame: {e}")))?;
            let angle_about = |z: Direction, p: Point| -> f64 {
                let y = Direction::new(z.cross_vector(x_dir), tol).expect("perpendicular axes");
                let d = p - center;
                let a = d.dot(y.vector()).atan2(d.dot(x_dir.vector()));
                if a <= 0.0 {
                    a + TAU
                } else {
                    a
                }
            };
            let n = plane_normal(plane).map_err(|e| seg_err(&format!("arc normal: {e}")))?;
            let (frame_z, sweep) = {
                let am = angle_about(n, pm);
                let ae = angle_about(n, p2);
                if am < ae {
                    (n, ae)
                } else {
                    let flipped = n.reversed();
                    (flipped, angle_about(flipped, p2))
                }
            };
            let frame = Frame::new(center, frame_z, x_dir, tol)
                .map_err(|e| seg_err(&format!("arc frame: {e}")))?;
            let circle = Circle::new(frame, radius, tol)
                .map_err(|e| seg_err(&format!("arc circle: {e}")))?;
            Ok((Curve::Circle(CircleCurve::new(circle)), (0.0, sweep)))
        }
        ProfileSegment::Circle { center, radius } => {
            if *radius <= 0.0 {
                return Err(seg_err("circle radius must be positive"));
            }
            let c = world_point(plane, center[0], center[1]);
            let frame = Frame::new(c, normal, x_axis, tol)
                .map_err(|e| seg_err(&format!("circle frame: {e}")))?;
            let circle =
                Circle::new(frame, *radius, tol).map_err(|e| seg_err(&format!("circle: {e}")))?;
            Ok((Curve::Circle(CircleCurve::new(circle)), (0.0, TAU)))
        }
        ProfileSegment::Ellipse {
            center,
            major,
            ratio,
        }
        | ProfileSegment::EllipseArc {
            center,
            major,
            ratio,
            ..
        } => {
            let major_len = major[0].hypot(major[1]);
            if major_len <= 0.0 || *ratio <= 0.0 || *ratio > 1.0 {
                return Err(seg_err(
                    "ellipse needs a positive major axis and ratio in (0, 1]",
                ));
            }
            let c = world_point(plane, center[0], center[1]);
            let major_dir = Direction::new(world_vector(plane, major[0], major[1]), tol)
                .map_err(|e| seg_err(&format!("ellipse major axis: {e}")))?;
            let frame = Frame::new(c, normal, major_dir, tol)
                .map_err(|e| seg_err(&format!("ellipse frame: {e}")))?;
            let ellipse = Ellipse::new(frame, major_len, major_len * ratio, tol)
                .map_err(|e| seg_err(&format!("ellipse: {e}")))?;
            let range = match seg {
                ProfileSegment::EllipseArc {
                    start_param,
                    end_param,
                    ..
                } => {
                    let start = *start_param;
                    let mut end = *end_param;
                    while end <= start {
                        end += TAU;
                    }
                    (start, end)
                }
                _ => (0.0, TAU),
            };
            Ok((Curve::Ellipse(EllipseCurve::new(ellipse)), range))
        }
        ProfileSegment::BSpline {
            control_points,
            periodic,
        } => {
            let n = control_points.len();
            if n < 2 || (*periodic && n < 3) {
                return Err(seg_err(
                    "B-spline needs at least 2 control points (3 when periodic)",
                ));
            }
            let poles: Vec<Point> = control_points
                .iter()
                .map(|p| world_point(plane, p[0], p[1]))
                .collect();
            let (curve, range) = if *periodic {
                // ogeom has no public periodic constructor; an unclamped
                // uniform spline over the wrapped control polygon is the same
                // curve over [degree, n + degree].
                let degree = 3usize.min(n - 1).max(1);
                let mut wrapped = poles.clone();
                wrapped.extend_from_slice(&poles[..degree.min(poles.len())]);
                let knot_count = wrapped.len() + degree + 1;
                let knots: Vec<f64> = (0..knot_count).map(|i| i as f64).collect();
                let kv = KnotVector::new(knots, degree)
                    .map_err(|e| seg_err(&format!("periodic B-spline knots: {e}")))?;
                let curve = BSplineCurve::new(kv, wrapped, tol)
                    .map_err(|e| seg_err(&format!("periodic B-spline: {e}")))?;
                (curve, (degree as f64, (n + degree) as f64))
            } else {
                let degree = 3usize.min(n - 1);
                let kv = KnotVector::clamped_uniform(degree, n)
                    .map_err(|e| seg_err(&format!("B-spline knots: {e}")))?;
                let range = kv.domain();
                let curve = BSplineCurve::new(kv, poles, tol)
                    .map_err(|e| seg_err(&format!("B-spline: {e}")))?;
                (curve, range)
            };
            Ok((Curve::BSpline(curve), range))
        }
    }
}

/// A segment's 2D endpoints (start, end), None when the segment closes on
/// itself (full circle/ellipse, periodic spline).
fn segment_endpoints(seg: &ProfileSegment) -> Option<([f64; 2], [f64; 2])> {
    match seg {
        ProfileSegment::Line { start, end } | ProfileSegment::Arc { start, end, .. } => {
            Some((*start, *end))
        }
        ProfileSegment::Circle { .. } | ProfileSegment::Ellipse { .. } => None,
        ProfileSegment::EllipseArc {
            center,
            major,
            ratio,
            start_param,
            end_param,
        } => {
            let at = |t: f64| {
                let (c, s) = (t.cos(), t.sin());
                let minor = [-major[1] * ratio, major[0] * ratio];
                [
                    center[0] + major[0] * c + minor[0] * s,
                    center[1] + major[1] * c + minor[1] * s,
                ]
            };
            Some((at(*start_param), at(*end_param)))
        }
        ProfileSegment::BSpline {
            control_points,
            periodic,
        } => {
            if *periodic {
                None
            } else {
                Some((*control_points.first()?, *control_points.last()?))
            }
        }
    }
}

/// One wire from a profile wire description. `require_closed` is relaxed for
/// spine paths.
///
/// Consecutive segments share endpoint *vertices* — ogeom wires connect by
/// shared topology, not coincident positions, the same invariant printCAD's
/// sketch endpoint snapping maintains.
pub fn build_wire(
    model: &mut Model,
    plane: &ProfilePlane,
    wire: &ProfileWire,
    require_closed: bool,
) -> Result<Shape, String> {
    build_wire_edges(model, plane, wire, require_closed).map(|b| b.wire)
}

/// As [`build_wire`], also returning the edge list.
pub fn build_wire_edges(
    model: &mut Model,
    plane: &ProfilePlane,
    wire: &ProfileWire,
    require_closed: bool,
) -> Result<BuiltWire, String> {
    let tol = tess::tolerances();
    if wire.segments.is_empty() {
        return Err("profile wire has no segments".into());
    }
    let normal = plane_normal(plane).map_err(|e| format!("profile plane: {e}"))?;
    let x_axis = Direction::new(
        Vector::new(plane.x_axis[0], plane.x_axis[1], plane.x_axis[2]),
        tol,
    )
    .map_err(|e| format!("profile plane x-axis: {e}"))?;

    // A single self-closing segment (circle, ellipse, periodic spline) makes
    // its own wire; make_edge names one vertex twice for a closed curve.
    if wire.segments.len() == 1 && segment_endpoints(&wire.segments[0]).is_none() {
        let (curve, range) = segment_curve(plane, normal, x_axis, &wire.segments[0])?;
        let edge = make_edge(model, curve, range, tol)
            .map(|b| b.shape)
            .map_err(|e| seg_err(&format!("closed segment edge: {e}")))?;
        return make_wire(model, std::slice::from_ref(&edge), tol)
            .map(|b| BuiltWire {
                wire: b.shape,
                edges: vec![edge],
            })
            .map_err(|e| format!("profile wire construction failed: {e}"));
    }

    let endpoints: Vec<([f64; 2], [f64; 2])> = wire
        .segments
        .iter()
        .map(|seg| {
            segment_endpoints(seg).ok_or_else(|| {
                "a self-closing segment must be the only one in its wire".to_string()
            })
        })
        .collect::<Result<_, _>>()?;

    let close = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-6;
    let n = wire.segments.len();
    let first_start = endpoints[0].0;
    let loops_back = close(endpoints[n - 1].1, first_start);
    if require_closed && !loops_back {
        return Err("profile wire is not closed".into());
    }

    let first_vertex = model.add_vertex(VertexData::new(world_point(
        plane,
        first_start[0],
        first_start[1],
    )));
    let mut edges = Vec::with_capacity(n);
    let mut start_vertex = first_vertex.clone();
    for (i, seg) in wire.segments.iter().enumerate() {
        if i > 0 && !close(endpoints[i].0, endpoints[i - 1].1) {
            return Err("profile segments do not form a connected wire".into());
        }
        let end_uv = endpoints[i].1;
        let end_vertex = if i == n - 1 && loops_back {
            first_vertex.clone()
        } else {
            model.add_vertex(VertexData::new(world_point(plane, end_uv[0], end_uv[1])))
        };
        let (curve, range) = segment_curve(plane, normal, x_axis, seg)?;
        let edge = make_edge_between(model, curve, range, &start_vertex, &end_vertex, tol)
            .map(|b| b.shape)
            .map_err(|e| seg_err(&format!("edge {i}: {e}")))?;
        edges.push(edge);
        start_vertex = end_vertex;
    }

    make_wire(model, &edges, tol)
        .map(|b| BuiltWire {
            wire: b.shape,
            edges,
        })
        .map_err(|e| format!("profile segments do not form a connected wire: {e}"))
}

/// Lift a whole profile: wires → containment groups → one face per group.
pub fn build_profile(model: &mut Model, profile: &Profile) -> Result<BuiltProfile, String> {
    let tol = tess::tolerances();
    if profile.wires.is_empty() {
        return Err("profile has no wires".into());
    }
    let plane = plane_of(&profile.plane).map_err(|e| format!("profile plane: {e}"))?;

    let mut wires = Vec::with_capacity(profile.wires.len());
    for w in &profile.wires {
        wires.push(build_wire_edges(model, &profile.plane, w, true)?);
    }

    // Area and a probe point per wire, via a throwaway single-wire face.
    let mut areas = Vec::with_capacity(wires.len());
    let mut solo_faces = Vec::with_capacity(wires.len());
    for (wire, desc) in wires.iter().zip(&profile.wires) {
        let surface = plane_surface(&plane, std::slice::from_ref(desc))
            .map_err(|e| format!("profile plane surface: {e}"))?;
        let face = make_face_with_pcurves(
            model,
            SurfaceGeometry::Plane(surface),
            std::slice::from_ref(&wire.edges),
            tol,
        )
        .map_err(|e| format!("profile face: {e}"))?
        .shape;
        let area = surface_properties(model, &face, Deflection::default(), tol)
            .map_err(|e| format!("profile wire area: {e}"))?
            .mass;
        if area <= 0.0 || !area.is_finite() {
            return Err("a profile wire encloses no area".into());
        }
        areas.push(area);
        solo_faces.push(face);
    }

    // First vertex of each wire as its containment probe.
    let probes: Vec<Point> = profile
        .wires
        .iter()
        .map(|w| first_point(&profile.plane, w))
        .collect::<Result<_, String>>()?;

    // Immediate container = smallest-area wire strictly containing this one.
    let n = wires.len();
    let mut parent: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        for j in 0..n {
            if i == j || areas[j] <= areas[i] {
                continue;
            }
            let inside =
                classify_on_face(model, &solo_faces[j], probes[i], Deflection::default(), tol)
                    .map(|c| c == ogeom::algo::Containment::In)
                    .unwrap_or(false);
            if !inside {
                continue;
            }
            if parent[i].is_none_or(|p| areas[j] < areas[p]) {
                parent[i] = Some(j);
            }
        }
    }
    let depth_of = |mut i: usize| {
        let mut d = 0usize;
        while let Some(p) = parent[i] {
            d += 1;
            i = p;
        }
        d
    };

    let mut groups: Vec<WireGroup> = Vec::new();
    let mut group_of: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        if depth_of(i) % 2 == 0 {
            group_of[i] = Some(groups.len());
            groups.push(WireGroup {
                outer: wires[i].clone(),
                holes: Vec::new(),
                wire_indices: vec![i],
            });
        }
    }
    for i in 0..n {
        if depth_of(i) % 2 == 1 {
            let container = parent[i].expect("odd depth implies a container");
            let g = group_of[container].expect("container is an outer wire");
            groups[g].holes.push(wires[i].clone());
            groups[g].wire_indices.push(i);
        }
    }

    let mut faces = Vec::with_capacity(groups.len());
    for group in &groups {
        let mut rings: Vec<Vec<Shape>> = Vec::with_capacity(1 + group.holes.len());
        rings.push(group.outer.edges.clone());
        rings.extend(group.holes.iter().map(|h| h.edges.clone()));
        let group_wires: Vec<ProfileWire> = group
            .wire_indices
            .iter()
            .map(|&i| profile.wires[i].clone())
            .collect();
        let surface = plane_surface(&plane, &group_wires)
            .map_err(|e| format!("profile plane surface: {e}"))?;
        let face = make_face_with_pcurves(model, SurfaceGeometry::Plane(surface), &rings, tol)
            .map_err(|e| format!("profile region face: {e}"))?
            .shape;
        faces.push(face);
    }

    Ok(BuiltProfile { faces, groups })
}

/// The first on-curve point a wire mentions, in world space.
fn first_point(plane: &ProfilePlane, wire: &ProfileWire) -> Result<Point, String> {
    let seg = wire
        .segments
        .first()
        .ok_or_else(|| "profile wire has no segments".to_string())?;
    let uv = match seg {
        ProfileSegment::Line { start, .. } | ProfileSegment::Arc { start, .. } => *start,
        ProfileSegment::Circle { center, radius } => [center[0] + radius, center[1]],
        ProfileSegment::Ellipse { center, major, .. } => {
            [center[0] + major[0], center[1] + major[1]]
        }
        ProfileSegment::EllipseArc {
            center,
            major,
            ratio,
            start_param,
            ..
        } => {
            let (c, s) = (start_param.cos(), start_param.sin());
            let minor = [-major[1] * ratio, major[0] * ratio];
            [
                center[0] + major[0] * c + minor[0] * s,
                center[1] + major[1] * c + minor[1] * s,
            ]
        }
        ProfileSegment::BSpline { control_points, .. } => *control_points
            .first()
            .ok_or_else(|| "B-spline has no control points".to_string())?,
    };
    Ok(world_point(plane, uv[0], uv[1]))
}

/// The area centroid of a built profile (all regions together), used as the
/// ray origin for termination queries.
pub fn profile_centroid(model: &Model, built: &BuiltProfile) -> Result<Point, String> {
    let tol = tess::tolerances();
    let mut weighted = Vector::new(0.0, 0.0, 0.0);
    let mut total = 0.0;
    for face in &built.faces {
        let props = surface_properties(model, face, Deflection::default(), tol)
            .map_err(|e| format!("profile centroid: {e}"))?;
        weighted += (props.centre - Point::new(0.0, 0.0, 0.0)) * props.mass;
        total += props.mass;
    }
    if total <= 0.0 {
        return Err("profile encloses no area".into());
    }
    Ok(Point::new(
        weighted.x / total,
        weighted.y / total,
        weighted.z / total,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rectangle_wire_chains_and_closes() {
        let mut model = Model::with_tolerances(tess::tolerances());
        let plane = ProfilePlane {
            origin: [0.0, 0.0, 0.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 1.0, 0.0],
            normal: [0.0, 0.0, 1.0],
        };
        let wire = ProfileWire {
            segments: vec![
                ProfileSegment::Line {
                    start: [0.0, 0.0],
                    end: [10.0, 0.0],
                },
                ProfileSegment::Line {
                    start: [10.0, 0.0],
                    end: [10.0, 5.0],
                },
                ProfileSegment::Line {
                    start: [10.0, 5.0],
                    end: [0.0, 5.0],
                },
                ProfileSegment::Line {
                    start: [0.0, 5.0],
                    end: [0.0, 0.0],
                },
            ],
        };
        let shape = build_wire(&mut model, &plane, &wire, true).expect("rectangle wire");
        assert!(ogeom::algo::is_wire_closed(&model, &shape, tess::tolerances()).unwrap());
    }
}
