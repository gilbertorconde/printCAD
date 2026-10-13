//! What a package asks of a body's geometry (`geom.*` calls), and the
//! cosmetic threads its plans declare, drawn on the faces they name.
//!
//! The kernel answers in the body's own frame; the names come from the
//! body's mesh, which records each face's name and the two faces each
//! outline edge runs between, matched to the kernel's answer by where it
//! lies.

use bench_api::{CosmeticThread, calls};
use core_document::{BodyPlacement, EdgeCircle, WorkbenchRuntimeContext};
use glam::Vec3;
use kernel_api::{FaceSurface, ProfilePlane, TriMesh};
use serde_json::{Value, json};

use crate::host::{body_id, str_arg};

/// A read-only geometry question: the kernel asked of a body's solid in
/// its own frame, the answer put where the body sits.
pub(crate) fn call(
    ctx: &WorkbenchRuntimeContext<'static>,
    command: &str,
    args: &Value,
) -> Result<Value, String> {
    let kernel = ctx.kernel.ok_or("no kernel is at hand to answer")?;
    let text = str_arg(args, "body")?;
    let body = body_id(text)?;
    let document = &*ctx.document;
    let brep = document
        .imported_brep_blob(body)
        .ok_or_else(|| format!("body {text} has no solid shape"))?;
    let placement = document.body_placement(body);
    let mesh = document.imported_geometry(body).map(|g| &*g.mesh);
    let point = |p: [f64; 3]| placement.point(p.map(|v| v as f32));
    let direction = |d: [f64; 3]| placement.direction(d.map(|v| v as f32));
    let local = |p: [f32; 3]| placement.inverse().point(p).map(f64::from);
    match command {
        calls::GEOM_HOLES => {
            let (holes, _) = kernel.recognize_holes(brep).map_err(|e| e.to_string())?;
            Ok(Value::Array(
                holes
                    .into_iter()
                    .map(|h| {
                        json!({
                            "entry": point(h.entry),
                            "direction": direction(h.direction),
                            "diameter": h.diameter,
                            "depth": h.depth,
                            "through": h.through,
                            "drill_point_deg": h.drill_point_deg,
                        })
                    })
                    .collect(),
            ))
        }
        calls::GEOM_FACE_EDGES => {
            let at = vec3_arg(args, "point")?;
            let edges = kernel
                .face_edges(brep, local(at))
                .map_err(|e| e.to_string())?;
            Ok(Value::Array(
                edges
                    .into_iter()
                    .map(|(p, d)| json!({"point": point(p), "direction": direction(d)}))
                    .collect(),
            ))
        }
        calls::GEOM_SECTION => {
            let world = plane_arg(args)?;
            let plane = placed_plane(&world, &placement.inverse());
            let near = match args.get("face") {
                Some(Value::Null) | None => None,
                Some(_) => Some(local(vec3_arg(args, "face")?)),
            };
            let runs = kernel
                .section(brep, &plane, near)
                .map_err(|e| e.to_string())?;
            let loops = runs
                .into_iter()
                .map(|run| bench_api::SectionLoop {
                    closed: run.closed,
                    curves: run
                        .curves
                        .into_iter()
                        .map(|c| {
                            let points = world_points(&placement, &c.points);
                            let face = mesh.and_then(|m| {
                                let at = middle(&points)?;
                                face_name(m, nearest_face(m, at)?)
                            });
                            bench_api::SectionCurve {
                                curve: c.curve,
                                reversed: c.reversed,
                                points: points.iter().map(|p| p.map(f64::from)).collect(),
                                face,
                            }
                        })
                        .collect(),
                })
                .collect();
            serde_json::to_value(bench_api::Section {
                plane: world,
                loops,
            })
            .map_err(|e| e.to_string())
        }
        calls::GEOM_FACE_LOOPS => {
            let near = match args.get("point") {
                Some(Value::Null) | None => None,
                Some(_) => Some(vec3_arg(args, "point")?),
            };
            let name = args.get("name").and_then(Value::as_u64);
            let at = match (name, near) {
                (Some(name), near) => mesh
                    .and_then(|m| point_on_named(m, name, near.map(Vec3::from_array)))
                    .map(|p| p.to_array())
                    .ok_or_else(|| format!("body {text} has no face named {name}"))?,
                (None, Some(near)) => near,
                (None, None) => return Err("argument `point` or `name` is required".into()),
            };
            let name =
                name.or_else(|| mesh.and_then(|m| face_name(m, nearest_face(m, Vec3::from(at))?)));
            let loops = kernel
                .face_loops(brep, local(at))
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|edges| {
                    edges
                        .into_iter()
                        .map(|e| {
                            let points = world_points(&placement, &e.points);
                            bench_api::LoopEdge {
                                length: points
                                    .windows(2)
                                    .map(|w| f64::from(Vec3::from(w[0]).distance(Vec3::from(w[1]))))
                                    .sum(),
                                circle: EdgeCircle::fit(&points).map(|c| bench_api::EdgeCircle {
                                    center: c.center,
                                    normal: c.normal,
                                    radius: c.radius,
                                }),
                                seam: e.seam,
                                faces: mesh.and_then(|m| edge_faces(m, middle(&points)?)),
                                points: points.iter().map(|p| p.map(f64::from)).collect(),
                            }
                        })
                        .collect()
                })
                .collect();
            serde_json::to_value(bench_api::FaceLoops { name, loops }).map_err(|e| e.to_string())
        }
        other => Err(format!("no geometry question `{other}`")),
    }
}

fn vec3_arg(args: &Value, name: &str) -> Result<[f32; 3], String> {
    let at: Vec<f32> = args
        .get(name)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_f64)
                .map(|v| v as f32)
                .collect()
        })
        .unwrap_or_default();
    match at[..] {
        [x, y, z] if at.iter().all(|v| v.is_finite()) => Ok([x, y, z]),
        _ => Err(format!("argument `{name}` must be [x, y, z]")),
    }
}

/// The `plane` argument: an origin and a normal, and the direction its
/// 2D coordinates run along when given (square to the normal), else one
/// chosen square to it.
fn plane_arg(args: &Value) -> Result<ProfilePlane, String> {
    let plane = args.get("plane").ok_or("argument `plane` is required")?;
    let origin = Vec3::from(vec3_arg(plane, "origin").map_err(|e| format!("plane: {e}"))?);
    let normal = Vec3::from(vec3_arg(plane, "normal").map_err(|e| format!("plane: {e}"))?)
        .try_normalize()
        .ok_or("plane: `normal` has no length")?;
    let given = match plane.get("x_axis") {
        Some(Value::Null) | None => None,
        Some(_) => Some(Vec3::from(
            vec3_arg(plane, "x_axis").map_err(|e| format!("plane: {e}"))?,
        )),
    };
    let x = given
        .unwrap_or_else(|| normal.any_orthonormal_vector())
        .reject_from_normalized(normal)
        .try_normalize()
        .ok_or("plane: `x_axis` runs along the normal")?;
    let y = normal.cross(x);
    let f = |v: Vec3| v.to_array().map(f64::from);
    Ok(ProfilePlane {
        origin: f(origin),
        x_axis: f(x),
        y_axis: f(y),
        normal: f(normal),
    })
}

/// `plane` moved by `placement`, its 2D coordinates kept.
fn placed_plane(plane: &ProfilePlane, placement: &BodyPlacement) -> ProfilePlane {
    let point = |p: [f64; 3]| placement.point(p.map(|c| c as f32)).map(f64::from);
    let turn = |d: [f64; 3]| placement.direction(d.map(|c| c as f32)).map(f64::from);
    ProfilePlane {
        origin: point(plane.origin),
        x_axis: turn(plane.x_axis),
        y_axis: turn(plane.y_axis),
        normal: turn(plane.normal),
    }
}

fn world_points(placement: &BodyPlacement, points: &[[f64; 3]]) -> Vec<[f32; 3]> {
    points
        .iter()
        .map(|p| placement.point(p.map(|v| v as f32)))
        .collect()
}

/// Halfway along a polyline's middle piece: on the curve for a straight
/// one, and away from the ends a neighbouring curve shares.
fn middle(points: &[[f32; 3]]) -> Option<Vec3> {
    let i = points.len().checked_sub(2)? / 2;
    Some((Vec3::from(points[i]) + Vec3::from(points[i + 1])) * 0.5)
}

/// The kernel face of `mesh` whose triangles pass nearest `p`.
pub(crate) fn nearest_face(mesh: &TriMesh, p: Vec3) -> Option<u32> {
    let mut best: Option<(f32, u32)> = None;
    for (t, face) in mesh.faces.iter().enumerate() {
        let corner = |k: usize| Vec3::from(mesh.positions[mesh.indices[3 * t + k] as usize]);
        let d = triangle_distance_sq(p, corner(0), corner(1), corner(2));
        if best.is_none_or(|(b, _)| d < b) {
            best = Some((d, *face));
        }
    }
    best.map(|(_, face)| face)
}

fn face_name(mesh: &TriMesh, face: u32) -> Option<u64> {
    mesh.face_names
        .get(face as usize)
        .copied()
        .filter(|n| *n != 0)
}

/// A point on the face named `name`: of several (a face a later feature
/// split), the one nearest `near`.
fn point_on_named(mesh: &TriMesh, name: u64, near: Option<Vec3>) -> Option<Vec3> {
    let mut best: Option<(f32, Vec3)> = None;
    for (t, face) in mesh.faces.iter().enumerate() {
        if face_name(mesh, *face) != Some(name) {
            continue;
        }
        let corner = |k: usize| Vec3::from(mesh.positions[mesh.indices[3 * t + k] as usize]);
        let (a, b, c) = (corner(0), corner(1), corner(2));
        let centre = (a + b + c) / 3.0;
        let d = near.map_or(0.0, |n| triangle_distance_sq(n, a, b, c));
        if best.is_none_or(|(b, _)| d < b) {
            best = Some((d, centre));
        }
        if near.is_none() {
            break;
        }
    }
    best.map(|(_, p)| p)
}

/// The names of the two faces the outline edge of `mesh` nearest `p` runs
/// between.
fn edge_faces(mesh: &TriMesh, p: Vec3) -> Option<[u64; 2]> {
    let mut best: Option<(f32, u32)> = None;
    for (pair, id) in mesh.edges.as_chunks::<2>().0.iter().zip(&mesh.edge_ids) {
        let a = Vec3::from(mesh.positions[pair[0] as usize]);
        let b = Vec3::from(mesh.positions[pair[1] as usize]);
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.length_squared().max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
        let d = (a + ab * t).distance_squared(p);
        if best.is_none_or(|(b, _)| d < b) {
            best = Some((d, *id));
        }
    }
    let names = *mesh.edge_faces.get(best?.1 as usize)?;
    names.iter().all(|n| *n != 0).then_some(names)
}

/// The squared distance from `p` to the triangle `a b c`.
fn triangle_distance_sq(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return ap.length_squared();
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return bp.length_squared();
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (a + ab * v).distance_squared(p);
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return cp.length_squared();
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (a + ac * w).distance_squared(p);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (b + (c - b) * w).distance_squared(p);
    }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    (a + ab * v + ac * w).distance_squared(p)
}

/// How many line pieces a cosmetic thread draws per turn.
const THREAD_SEGMENTS_PER_TURN: f32 = 24.0;

/// Past this many pieces a thread is not drawn: a pitch far too fine for
/// its length.
const THREAD_MOST_PIECES: f32 = 100_000.0;

/// How far off its face a thread is drawn, as a part of the radius, so
/// the face does not hide it.
const THREAD_CLEARANCE: f32 = 0.02;

/// `thread` as a helix over its round face of `mesh` (both in the body's
/// frame): the face found by name, else the one at its point, which must
/// be a circular cylinder. Empty when it is not, or the thread has no
/// pitch or length.
pub(crate) fn thread_helix(mesh: &TriMesh, thread: &CosmeticThread) -> Vec<[f32; 3]> {
    let at = Vec3::from_array(thread.point.map(|v| v as f32));
    let face = thread
        .name
        .and_then(|name| point_on_named(mesh, name, Some(at)))
        .and_then(|p| nearest_face(mesh, p))
        .or_else(|| nearest_face(mesh, at));
    let Some(face) = face else {
        return Vec::new();
    };
    let Some(FaceSurface::Cylinder {
        origin,
        axis,
        radius,
    }) = mesh.face_surfaces.get(face as usize).copied()
    else {
        return Vec::new();
    };
    let (origin, axis) = (Vec3::from(origin), Vec3::from(axis).normalize_or_zero());
    let pitch = thread.pitch as f32;
    if axis == Vec3::ZERO || radius <= 0.0 || !pitch.is_finite() || pitch <= 0.0 {
        return Vec::new();
    }
    // How far along the axis the face runs, and which way it faces.
    let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
    let mut facing = 0.0;
    for (t, f) in mesh.faces.iter().enumerate() {
        if *f != face {
            continue;
        }
        for k in 0..3 {
            let i = mesh.indices[3 * t + k] as usize;
            let p = Vec3::from(mesh.positions[i]);
            let along = (p - origin).dot(axis);
            low = low.min(along);
            high = high.max(along);
            if let Some(n) = mesh.normals.get(i) {
                facing += Vec3::from(*n).dot((p - origin).reject_from_normalized(axis));
            }
        }
    }
    if high <= low {
        return Vec::new();
    }
    // A bore faces its axis: the thread goes just inside it.
    let drawn = if facing < 0.0 {
        radius * (1.0 - THREAD_CLEARANCE)
    } else {
        radius * (1.0 + THREAD_CLEARANCE)
    };
    let along = (at - origin).dot(axis);
    let (start, into) = if along - low <= high - along {
        (low, axis)
    } else {
        (high, -axis)
    };
    let whole = high - low;
    let length = match thread.length as f32 {
        l if l > 0.0 => l.min(whole),
        _ => whole,
    };
    let pieces = (length / pitch * THREAD_SEGMENTS_PER_TURN).ceil();
    if !pieces.is_finite() || !(1.0..=THREAD_MOST_PIECES).contains(&pieces) {
        return Vec::new();
    }
    let base = origin + axis * start;
    let u = into.any_orthonormal_vector();
    let v = into.cross(u);
    let turn = if thread.left_handed { -1.0 } else { 1.0 };
    let pieces = pieces as usize;
    (0..=pieces)
        .map(|i| {
            let down = length * i as f32 / pieces as f32;
            let angle = turn * std::f32::consts::TAU * down / pitch;
            let (sin, cos) = angle.sin_cos();
            (base + into * down + drawn * (cos * u + sin * v)).to_array()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tube's two round faces, as a mesh carries them: an outside of
    /// radius 5 facing out (face 0, named 11) and a bore of radius 3 facing
    /// in (face 1, named 22), both from z = 0 to 10 about the Z axis.
    fn tube() -> TriMesh {
        let mut mesh = TriMesh::default();
        for (face, radius, out) in [(0u32, 5.0f32, 1.0f32), (1, 3.0, -1.0)] {
            let base = mesh.positions.len() as u32;
            let sides = 24;
            for i in 0..sides {
                let a = i as f32 * std::f32::consts::TAU / sides as f32;
                let (s, c) = a.sin_cos();
                for z in [0.0, 10.0] {
                    mesh.positions.push([radius * c, radius * s, z]);
                    mesh.normals.push([out * c, out * s, 0.0]);
                }
            }
            for i in 0..sides {
                let (a, b) = (base + 2 * i, base + 2 * ((i + 1) % sides));
                mesh.indices.extend([a, b, a + 1, b, b + 1, a + 1]);
                mesh.faces.extend([face, face]);
            }
            mesh.face_surfaces.push(FaceSurface::Cylinder {
                origin: [0.0; 3],
                axis: [0.0, 0.0, 1.0],
                radius,
            });
        }
        mesh.face_names = vec![11, 22];
        mesh
    }

    fn radius_of(p: &[f32; 3]) -> f32 {
        (p[0] * p[0] + p[1] * p[1]).sqrt()
    }

    /// A thread on a bore is drawn just inside it, one helix of its pitch
    /// down the face from the end nearest its point; on a shank, just
    /// outside, and as long as asked.
    #[test]
    fn a_thread_winds_down_its_round_face_inside_a_bore_and_outside_a_shank() {
        let mesh = tube();
        let bore = CosmeticThread {
            point: [3.0, 0.0, 9.0],
            pitch: 1.0,
            ..Default::default()
        };
        let helix = thread_helix(&mesh, &bore);
        assert_eq!(helix.len(), 10 * 24 + 1, "ten turns");
        assert!(helix.iter().all(|p| {
            let r = radius_of(p);
            r < 3.0 && r > 2.9
        }));
        assert!((helix[0][2] - 10.0).abs() < 1e-4, "from the near end");
        assert!(helix[helix.len() - 1][2].abs() < 1e-4);
        // Each turn comes back round to where it began.
        let (a, b) = (Vec3::from(helix[0]), Vec3::from(helix[24]));
        assert!(((a - b).length() - 1.0).abs() < 1e-3);

        let shank = CosmeticThread {
            point: [0.0, 5.0, 1.0],
            pitch: 0.5,
            length: 4.0,
            ..Default::default()
        };
        let helix = thread_helix(&mesh, &shank);
        assert!(
            helix
                .iter()
                .all(|p| radius_of(p) > 5.0 && radius_of(p) < 5.2)
        );
        assert!(helix[0][2].abs() < 1e-4);
        assert!((helix[helix.len() - 1][2] - 4.0).abs() < 1e-4);
    }

    /// A name finds its face wherever the point now lies; a face that is
    /// not a cylinder, or no pitch, draws nothing.
    #[test]
    fn a_thread_finds_its_face_by_name_and_draws_nothing_off_a_cylinder() {
        let mut mesh = tube();
        let named = CosmeticThread {
            point: [5.0, 0.0, 0.0],
            name: Some(22),
            pitch: 1.0,
            ..Default::default()
        };
        assert!(
            thread_helix(&mesh, &named)
                .iter()
                .all(|p| radius_of(p) < 3.0)
        );
        let flat = CosmeticThread {
            pitch: 0.0,
            ..named.clone()
        };
        assert!(thread_helix(&mesh, &flat).is_empty());
        mesh.face_surfaces[1] = FaceSurface::Plane {
            origin: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
        };
        assert!(thread_helix(&mesh, &named).is_empty());
    }

    /// A point finds the face its triangles pass nearest, and that face's
    /// name; a name finds a point on its face.
    #[test]
    fn faces_are_found_by_point_and_by_name() {
        let mesh = tube();
        assert_eq!(nearest_face(&mesh, Vec3::new(0.0, 3.1, 5.0)), Some(1));
        assert_eq!(face_name(&mesh, 0), Some(11));
        let on = point_on_named(&mesh, 11, None).expect("a point on the outside");
        assert_eq!(nearest_face(&mesh, on), Some(0));
        assert!(point_on_named(&mesh, 99, None).is_none());
    }

    /// An outline edge near a point gives the names of its two faces, none
    /// where the mesh names them not.
    #[test]
    fn an_edge_is_named_by_its_two_faces() {
        let mut mesh = TriMesh {
            positions: vec![[0.0; 3], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0]],
            edges: vec![0, 1, 1, 2],
            edge_ids: vec![0, 1],
            edge_faces: vec![[1, 2], [2, 0]],
            ..Default::default()
        };
        assert_eq!(edge_faces(&mesh, Vec3::new(5.0, 0.1, 0.0)), Some([1, 2]));
        assert_eq!(edge_faces(&mesh, Vec3::new(10.0, 6.0, 0.0)), None);
        mesh.edge_faces.clear();
        assert_eq!(edge_faces(&mesh, Vec3::new(5.0, 0.1, 0.0)), None);
    }

    /// A plane given by its origin and normal alone gets axes square to
    /// it; a body's frame moves it keeping its 2D coordinates.
    #[test]
    fn a_plane_argument_reads_with_or_without_its_axis() {
        let plane = plane_arg(&json!({"plane": {"origin": [0, 0, 4], "normal": [0, 0, 2]}}))
            .expect("reads");
        let n = Vec3::from(plane.normal.map(|v| v as f32));
        let x = Vec3::from(plane.x_axis.map(|v| v as f32));
        let y = Vec3::from(plane.y_axis.map(|v| v as f32));
        assert!((n - Vec3::Z).length() < 1e-6);
        assert!(x.dot(n).abs() < 1e-6 && (x.cross(y) - n).length() < 1e-6);
        let given = plane_arg(&json!({"plane": {
            "origin": [0, 0, 0], "normal": [0, 0, 1], "x_axis": [1, 0, 0.5]
        }}))
        .unwrap();
        assert_eq!(given.x_axis, [1.0, 0.0, 0.0]);
        assert!(plane_arg(&json!({"plane": {"origin": [0, 0, 0], "normal": [0, 0, 0]}})).is_err());
        assert!(plane_arg(&json!({})).is_err());
    }
}
