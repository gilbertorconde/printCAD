//! Extrude / revolve / helix tools from a sketch profile.
//!
//! Terminations mirror the previous kernel: blind prisms, through-all lengths
//! derived from the base bounding box, up-to-plane via half-space trims, and
//! to-first/to-last via a ray query against the base solid's faces.

use kernel_api::{ExtrudeTermination, Profile, SweepKind};
use ogeom::algo::{make_natural_face, make_prism, make_prism_tapered, make_revolution};
use ogeom::geom::{Curve, Curve3d, HelixCurve, PlaneSurface, SurfaceGeometry};
use ogeom::math::{Axis, Direction, Frame, Plane, Point, Transform, Vector};
use ogeom::mesh::{triangulate_face, Deflection};
use ogeom::topo::{explore, Filter, Model, NodeData, Shape, ShapeType};

use super::{fuse_all, tol};
use crate::profile::{self, BuiltProfile};
use crate::tess;

const TAU: f64 = std::f64::consts::TAU;

pub fn build_tool(
    model: &mut Model,
    base: Option<&Shape>,
    prof: &Profile,
    kind: &SweepKind,
) -> Result<Shape, String> {
    let built = profile::build_profile(model, prof)?;
    match kind {
        SweepKind::Extrude {
            termination,
            second_side,
            symmetric,
            reversed,
            taper_deg,
            direction,
        } => extrude(
            model,
            base,
            prof,
            &built,
            termination,
            second_side.as_ref(),
            *symmetric,
            *reversed,
            *taper_deg,
            direction.as_ref(),
        ),
        SweepKind::Revolve {
            axis_origin,
            axis_dir,
            angle_deg,
            second_angle_deg,
            midplane,
            reversed,
        } => revolve(
            model,
            prof,
            &built,
            *axis_origin,
            *axis_dir,
            *angle_deg,
            *second_angle_deg,
            *midplane,
            *reversed,
        ),
        SweepKind::Helix {
            axis_origin,
            axis_dir,
            pitch,
            height,
            left_handed,
            cone_angle_deg,
            reversed,
        } => helix(
            model,
            prof,
            &built,
            *axis_origin,
            *axis_dir,
            *pitch,
            *height,
            *left_handed,
            *cone_angle_deg,
            *reversed,
        ),
    }
}

#[expect(clippy::too_many_arguments)]
fn extrude(
    model: &mut Model,
    base: Option<&Shape>,
    prof: &Profile,
    built: &BuiltProfile,
    termination: &ExtrudeTermination,
    second_side: Option<&ExtrudeTermination>,
    symmetric: bool,
    reversed: bool,
    taper_deg: f64,
    direction: Option<&[f64; 3]>,
) -> Result<Shape, String> {
    let normal = profile::plane_normal(&prof.plane).map_err(|e| format!("profile plane: {e}"))?;
    let mut dir = match direction {
        None => normal,
        Some(custom) => {
            let v = Vector::new(custom[0], custom[1], custom[2]);
            let d = Direction::new(v, tol())
                .map_err(|_| "custom extrusion direction is (near) zero".to_string())?;
            if d.dot(normal).abs() <= 1e-9 {
                return Err("custom extrusion direction is parallel to the sketch plane".into());
            }
            d
        }
    };
    if reversed {
        dir = dir.reversed();
    }

    let mut tool = extrude_one_side(model, base, built, dir, termination, taper_deg)?;
    if let Some(term2) = second_side {
        let back = extrude_one_side(model, base, built, dir.reversed(), term2, taper_deg)?;
        tool = ogeom::boolean::fuse(model, &tool, &back, tol())
            .map_err(|e| format!("fusing the two sweep sides failed: {e}"))?
            .shape;
    } else if symmetric {
        if let ExtrudeTermination::Blind { distance } = termination {
            let shift = Transform::translation(dir.vector() * (-distance * 0.5));
            tool = ogeom::algo::transformed(model, &tool, shift)
                .map_err(|e| format!("centering the symmetric extrusion failed: {e}"))?
                .shape;
        }
    }
    Ok(tool)
}

fn extrude_one_side(
    model: &mut Model,
    base: Option<&Shape>,
    built: &BuiltProfile,
    dir: Direction,
    term: &ExtrudeTermination,
    taper_deg: f64,
) -> Result<Shape, String> {
    match term {
        ExtrudeTermination::Blind { distance } => {
            prism_solid(model, built, dir, *distance, taper_deg)
        }
        ExtrudeTermination::ThroughAll => {
            let base = base.ok_or_else(|| {
                "a through-all extrusion needs existing material to pass through".to_string()
            })?;
            let centroid = profile::profile_centroid(model, built)?;
            let d = through_all_length(model, base, centroid, dir)?;
            prism_solid(model, built, dir, d, taper_deg)
        }
        ExtrudeTermination::UpToPlane {
            point,
            normal,
            offset,
        } => {
            let plane_normal = Direction::new(Vector::new(normal[0], normal[1], normal[2]), tol())
                .map_err(|_| "target plane normal is (near) zero".to_string())?;
            let plane_point =
                Point::new(point[0], point[1], point[2]) + plane_normal.vector() * *offset;
            up_to_plane(model, built, dir, plane_point, plane_normal, taper_deg)
        }
        ExtrudeTermination::ToFirst | ExtrudeTermination::ToLast => {
            let base = base.ok_or_else(|| {
                "a to-first/to-last extrusion needs existing material to stop at".to_string()
            })?;
            let centroid = profile::profile_centroid(model, built)?;
            let to_first = matches!(term, ExtrudeTermination::ToFirst);
            let hit = ray_hit(model, base, centroid, dir, to_first)?.ok_or_else(|| {
                "the extrusion direction does not hit the existing material".to_string()
            })?;
            match hit.plane {
                Some((p, n)) => up_to_plane(model, built, dir, p, n, taper_deg),
                None => prism_solid(model, built, dir, hit.distance, taper_deg),
            }
        }
    }
}

fn up_to_plane(
    model: &mut Model,
    built: &BuiltProfile,
    dir: Direction,
    plane_point: Point,
    plane_normal: Direction,
    taper_deg: f64,
) -> Result<Shape, String> {
    let centroid = profile::profile_centroid(model, built)?;
    let denom = dir.dot(plane_normal);
    if denom.abs() <= 1e-9 {
        return Err("the target plane is parallel to the extrusion direction".into());
    }
    let t = (plane_point - centroid).dot(plane_normal.vector()) / denom;
    if t <= 1e-9 {
        return Err("the target plane lies behind the sketch along the extrusion direction".into());
    }
    let diag = profile_diagonal(model, built);
    let reach = t + diag + 1.0;
    let long_prism = prism_solid(model, built, dir, reach, taper_deg)?;
    trim_with_halfspace(model, &long_prism, plane_point, plane_normal, centroid)
}

/// Keep only the material on `keep_point`'s side of the plane.
pub fn trim_with_halfspace(
    model: &mut Model,
    shape: &Shape,
    plane_point: Point,
    plane_normal: Direction,
    keep_point: Point,
) -> Result<Shape, String> {
    let plane = Plane::through(plane_point, plane_normal);
    let surface = PlaneSurface::over(plane, (-1.0e6, 1.0e6), (-1.0e6, 1.0e6))
        .map_err(|e| format!("trim plane surface: {e}"))?;
    let face = make_natural_face(model, SurfaceGeometry::Plane(surface))
        .map_err(|e| format!("trim plane face: {e}"))?
        .shape;
    let half = ogeom::algo::make_half_space(model, &face, keep_point, tol())
        .map_err(|e| format!("trim half-space: {e}"))?
        .shape;
    ogeom::boolean::common(model, shape, &half, tol())
        .map(|b| super::normalized(model, b.shape))
        .map_err(|e| format!("trimming the sweep at the target plane failed: {e}"))
}

fn prism_solid(
    model: &mut Model,
    built: &BuiltProfile,
    dir: Direction,
    distance: f64,
    taper_deg: f64,
) -> Result<Shape, String> {
    if distance <= 1e-9 || !distance.is_finite() {
        return Err("extrusion distance must be positive".into());
    }
    let sweep = dir.vector() * distance;
    let mut parts = Vec::with_capacity(built.faces.len());
    for face in &built.faces {
        let part = if taper_deg.abs() <= 1e-9 {
            make_prism(model, face, sweep, tol())
                .map_err(|e| format!("prism extrusion failed: {e}"))?
        } else {
            if taper_deg.abs() >= 89.9 {
                return Err("taper angle must be below 90 degrees".into());
            }
            make_prism_tapered(model, face, sweep, taper_deg.to_radians(), tol())
                .map_err(|e| format!("tapered prism failed: {e}"))?
        };
        parts.push(part.shape);
    }
    fuse_all(model, parts)
}

/// Length that guarantees a prism from `from` along `dir` passes fully
/// through the base solid.
fn through_all_length(
    model: &Model,
    base: &Shape,
    from: Point,
    dir: Direction,
) -> Result<f64, String> {
    let (lo, hi) = tess::robust_bounds(model, base)
        .ok_or_else(|| "the base solid has no bounds".to_string())?;
    let mut furthest = 0.0f64;
    for &x in &[lo.x, hi.x] {
        for &y in &[lo.y, hi.y] {
            for &z in &[lo.z, hi.z] {
                let to_corner = Point::new(x, y, z) - from;
                furthest = furthest.max(to_corner.dot(dir.vector()));
            }
        }
    }
    let diag = (hi - lo).magnitude();
    Ok(furthest.max(0.0) + diag * 0.1 + 1.0)
}

fn profile_diagonal(model: &Model, built: &BuiltProfile) -> f64 {
    built
        .faces
        .iter()
        .filter_map(|f| tess::robust_bounds(model, f))
        .map(|(lo, hi)| (hi - lo).magnitude())
        .fold(1.0, f64::max)
}

pub struct RayHit {
    pub distance: f64,
    /// Set when the hit face is planar: its world-space plane.
    pub plane: Option<(Point, Direction)>,
}

/// Nearest (or farthest) intersection of the ray with the base solid's
/// faces, via each face's triangulation. Mirrors the previous kernel's exact
/// intersector closely enough for termination queries.
pub fn ray_hit(
    model: &Model,
    base: &Shape,
    origin: Point,
    dir: Direction,
    nearest: bool,
) -> Result<Option<RayHit>, String> {
    let faces = explore(model, base, Filter::OfType(ShapeType::Face))
        .map_err(|e| format!("exploring base faces: {e}"))?;
    let mut best: Option<(f64, usize)> = None;
    for (i, face) in faces.iter().enumerate() {
        let Ok(tri) = triangulate_face(model, face, Deflection::default(), tol()) else {
            continue;
        };
        for t in &tri.triangles {
            let [a, b, c] = t.map(|i| tri.positions[i as usize]);
            if let Some(w) = ray_triangle(origin, dir, a, b, c) {
                if w > 1e-6 {
                    let better = match best {
                        None => true,
                        Some((bw, _)) => {
                            if nearest {
                                w < bw
                            } else {
                                w > bw
                            }
                        }
                    };
                    if better {
                        best = Some((w, i));
                    }
                }
            }
        }
    }
    let Some((distance, face_idx)) = best else {
        return Ok(None);
    };
    Ok(Some(RayHit {
        distance,
        plane: face_plane(model, &faces[face_idx]),
    }))
}

/// The world-space plane of a face whose carrier is planar.
pub fn face_plane(model: &Model, face: &Shape) -> Option<(Point, Direction)> {
    let node = model.node(face)?;
    let NodeData::Face(data) = node.data() else {
        return None;
    };
    let surface = model.geometry().surface(data.surface)?;
    let SurfaceGeometry::Plane(plane_surface) = surface else {
        return None;
    };
    let placement = face.transform(model.datums()).ok()?;
    let frame = placement
        .apply_frame(&plane_surface.plane().frame(), tol())
        .ok()?;
    Some((frame.origin(), frame.z()))
}

/// Möller–Trumbore; returns the ray parameter of the hit.
fn ray_triangle(origin: Point, dir: Direction, a: Point, b: Point, c: Point) -> Option<f64> {
    let e1 = b - a;
    let e2 = c - a;
    let d = dir.vector();
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - a;
    let u = s.dot(p) * inv;
    if !(-1e-9..=1.0 + 1e-9).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < -1e-9 || u + v > 1.0 + 1e-9 {
        return None;
    }
    Some(e2.dot(q) * inv)
}

#[expect(clippy::too_many_arguments)]
fn revolve(
    model: &mut Model,
    prof: &Profile,
    built: &BuiltProfile,
    axis_origin: [f64; 2],
    axis_dir: [f64; 2],
    angle_deg: f64,
    second_angle_deg: Option<f64>,
    midplane: bool,
    reversed: bool,
) -> Result<Shape, String> {
    let axis = sketch_plane_axis(&prof.plane, axis_origin, axis_dir)?;
    let axis = if reversed { axis_reversed(&axis) } else { axis };

    let (mut forward, mut backward) = (angle_deg, second_angle_deg.unwrap_or(0.0));
    if midplane {
        forward = angle_deg * 0.5;
        backward = angle_deg * 0.5;
    }
    let total = forward + backward;
    if total <= 0.0 || total > 360.0 + 1e-6 {
        return Err(format!(
            "revolve angle must be in (0, 360] degrees, got {total}"
        ));
    }
    let total_rad = if total >= 359.999 {
        TAU
    } else {
        total.to_radians()
    };

    // A full turn's seam position is arbitrary, but leaving it at the sketch
    // plane makes the seam edge coincide with any base face on that plane —
    // exactly the edge-on-face contact the boolean refuses. Park the seam at
    // an unaligned angle instead.
    let seam_offset = if total_rad >= TAU { 1.0 } else { 0.0 };

    let mut parts = Vec::with_capacity(built.faces.len());
    for face in &built.faces {
        let pre_angle = seam_offset - backward.to_radians();
        let sweep_face = if pre_angle != 0.0 {
            let pre = Transform::rotation(axis, pre_angle);
            ogeom::algo::transformed(model, face, pre)
                .map_err(|e| format!("pre-rotating the revolve profile failed: {e}"))?
                .shape
        } else {
            face.clone()
        };
        let part = make_revolution(model, &sweep_face, axis, total_rad, tol())
            .map_err(|e| format!("revolve operation failed: {e}"))?;
        parts.push(part.shape);
    }
    fuse_all(model, parts)
}

/// World-space axis from a sketch-plane (uv point, uv direction) pair.
pub fn sketch_plane_axis(
    plane: &kernel_api::ProfilePlane,
    origin_uv: [f64; 2],
    dir_uv: [f64; 2],
) -> Result<Axis, String> {
    let v = profile::world_vector(plane, dir_uv[0], dir_uv[1]);
    let d = Direction::new(v, tol())
        .map_err(|_| "axis direction is (near) zero in the sketch plane".to_string())?;
    Ok(Axis::new(
        profile::world_point(plane, origin_uv[0], origin_uv[1]),
        d,
    ))
}

fn axis_reversed(axis: &Axis) -> Axis {
    Axis::new(axis.location, axis.direction.reversed())
}

#[expect(clippy::too_many_arguments)]
fn helix(
    model: &mut Model,
    prof: &Profile,
    built: &BuiltProfile,
    axis_origin: [f64; 2],
    axis_dir: [f64; 2],
    pitch: f64,
    height: f64,
    left_handed: bool,
    cone_angle_deg: f64,
    reversed: bool,
) -> Result<Shape, String> {
    if pitch <= 1e-9 || height <= 1e-9 {
        return Err("helix pitch and height must be positive".into());
    }
    let axis = sketch_plane_axis(&prof.plane, axis_origin, axis_dir)?;
    let centroid = profile::profile_centroid(model, built)?;

    let to_start = centroid - axis.location;
    let along = axis.direction.vector() * to_start.dot(axis.direction.vector());
    let radial = to_start - along;
    let radius = radial.magnitude();
    if radius <= 1e-9 {
        return Err("the profile sits on the helix axis (zero radius)".into());
    }

    let mut axis_z = axis.direction;
    if reversed {
        axis_z = axis_z.reversed();
    }
    let base_point = centroid - radial;
    let radial_dir = Direction::new(radial, tol())
        .map_err(|_| "helix radial direction is degenerate".to_string())?;

    let frame = if left_handed {
        // Left-handed winding: flip the frame's y so "counter-clockwise about
        // z" turns the other way in world space.
        let y = Direction::new(-axis_z.cross_vector(radial_dir), tol())
            .map_err(|e| format!("helix frame: {e}"))?;
        Frame::from_axes(base_point, radial_dir, y, axis_z, tol())
            .map_err(|e| format!("helix frame: {e}"))?
    } else {
        Frame::new(base_point, axis_z, radial_dir, tol())
            .map_err(|e| format!("helix frame: {e}"))?
    };

    let turns = height / pitch;
    let curve = if cone_angle_deg.abs() > 1e-9 {
        // Radial advance per turn from the cone's half-angle:
        // tan(angle) = taper / pitch.
        let taper = pitch * cone_angle_deg.to_radians().tan();
        HelixCurve::conical(frame, radius, pitch, taper, 0.0, TAU * turns)
            .map_err(|e| format!("helix spine: {e}"))?
    } else {
        HelixCurve::new(frame, radius, pitch, turns).map_err(|e| format!("helix spine: {e}"))?
    };
    let range = curve.domain();
    let spine_edge = ogeom::algo::make_edge(model, Curve::Helix(curve), range, tol())
        .map_err(|e| format!("helix spine edge: {e}"))?
        .shape;
    let spine = ogeom::algo::make_wire(model, std::slice::from_ref(&spine_edge), tol())
        .map_err(|e| format!("helix spine wire: {e}"))?
        .shape;

    // The spine starts at the profile centroid; its tangent leans off the
    // sketch-plane normal by the helix lead angle. Rotate the profile about
    // the radial axis so it is square to the tangent, as the sweep requires.
    let tangent = spine_tangent(frame, radius, pitch);
    let normal = profile::plane_normal(&prof.plane).map_err(|e| format!("profile plane: {e}"))?;
    let aligned = align_to_tangent(normal, tangent, radial_dir);
    let rotate = Transform::rotation(Axis::new(centroid, radial_dir), aligned);

    let mut parts = Vec::with_capacity(built.faces.len());
    for face in &built.faces {
        let squared = ogeom::algo::transformed(model, face, rotate)
            .map_err(|e| format!("aligning the helix profile failed: {e}"))?
            .shape;
        let part = ogeom::offset::make_pipe_shell(model, &squared, &spine, false, 1e-3, tol())
            .map_err(|e| format!("helix sweep failed: {e}"))?;
        parts.push(part.shape);
    }
    fuse_all(model, parts)
}

/// Unit tangent of the helix at its start (t = 0).
fn spine_tangent(frame: Frame, radius: f64, pitch: f64) -> Vector {
    // d/dt [r(cos t · x + sin t · y) + pitch·t/2π · z] at t=0.
    let v = frame.y().vector() * radius + frame.z().vector() * (pitch / TAU);
    v * (1.0 / v.magnitude())
}

/// Signed angle about `axis` that rotates `normal` onto whichever of
/// ±`tangent` it is closer to.
fn align_to_tangent(normal: Direction, tangent: Vector, axis: Direction) -> f64 {
    let n = normal.vector();
    let t = if n.dot(tangent) >= 0.0 {
        tangent
    } else {
        -tangent
    };
    let sin = axis.vector().dot(n.cross(t));
    let cos = n.dot(t);
    sin.atan2(cos)
}
