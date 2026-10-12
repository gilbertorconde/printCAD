//! Extrude / revolve / helix tools from a sketch profile, or from a flat
//! face of the solid.
//!
//! Terminations: blind prisms, through-all lengths derived from the base
//! bounding box, and stops on a plane, on a face of the base (picked, or
//! the first or last a ray from the profile meets) or on a face of another
//! shape (a face borrowed from another body), each a long prism
//! trimmed by the half-space of the target's surface: a plane's, or a
//! curved face's whole surface, so a prism stops exactly on a cylinder, a
//! sphere or a spline. A stop on several faces takes from a long prism what
//! lies beyond each face along the sweep. A revolution stops on a flat face
//! whose plane holds its axis, turning exactly as far as that plane.

use kernel_api::{ExtrudeTermination, FaceProbe, Profile, RevolveTermination, SweepKind};
use ogeom::algo::{make_natural_face, make_prism, make_prism_tapered, make_revolution};
use ogeom::geom::{PlaneSurface, SurfaceGeometry, Transformable};
use ogeom::math::{Axis, Direction, Plane, Point, Transform, Vector};
use ogeom::mesh::{Deflection, triangulate_face};
use ogeom::offset::make_revolution_until;
use ogeom::topo::{Filter, Model, NodeData, Shape, ShapeType, explore};

use super::{fuse_all, tol};
use crate::profile::{self, BuiltProfile};
use crate::tess;

const TAU: f64 = std::f64::consts::TAU;

fn point3(p: [f64; 3]) -> Point {
    Point::new(p[0], p[1], p[2])
}

pub fn build_tool(
    model: &mut Model,
    base: Option<&Shape>,
    prof: &Profile,
    kind: &SweepKind,
) -> Result<Shape, String> {
    let built = profile::build_profile(model, prof)?;
    match kind {
        SweepKind::Extrude { .. } => {
            let normal =
                profile::plane_normal(&prof.plane).map_err(|e| format!("profile plane: {e}"))?;
            extrude(model, base, &built, normal, kind)
        }
        SweepKind::Revolve {
            axis_origin,
            axis_dir,
            angle_deg,
            second_angle_deg,
            midplane,
            reversed,
            termination,
        } => revolve(
            model,
            base,
            prof,
            &built,
            *axis_origin,
            *axis_dir,
            *angle_deg,
            *second_angle_deg,
            *midplane,
            *reversed,
            Stop::Termination(termination),
        ),
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
        } => {
            let extent = HelixExtent::of(*pitch, *height, *turns, *cone_angle_deg, *growth)?;
            helix(
                model,
                prof,
                &built,
                *axis_origin,
                *axis_dir,
                extent,
                *left_handed,
                *reversed,
            )
        }
        SweepKind::HelixNormal {
            axis_origin,
            pitch,
            height,
            left_handed,
            cone_angle_deg,
            reversed,
            turns,
            growth,
        } => {
            let extent = HelixExtent::of(*pitch, *height, *turns, *cone_angle_deg, *growth)?;
            let p = &prof.plane;
            let location = profile::world_point(p, axis_origin[0], axis_origin[1]);
            let normal = Direction::new(Vector::new(p.normal[0], p.normal[1], p.normal[2]), tol())
                .map_err(|_| "the sketch plane has no normal".to_string())?;
            helix_about(
                model,
                &built,
                Axis::new(location, normal),
                extent,
                *left_handed,
                *reversed,
            )
        }
    }
}

/// The tool a profile revolving until it meets the face at `point` of
/// `shape` (native format, moved by `transform`) makes: a face another
/// body lends, as a revolution's or groove's target.
pub fn build_revolve_to_face_of_tool(
    model: &mut Model,
    prof: &Profile,
    kind: &SweepKind,
    shape: &[u8],
    transform: Option<&[[f64; 4]; 4]>,
    point: [f64; 3],
) -> Result<Shape, String> {
    let SweepKind::Revolve {
        axis_origin,
        axis_dir,
        angle_deg,
        second_angle_deg,
        midplane,
        reversed,
        ..
    } = kind
    else {
        return Err("only a revolution turns until it meets a borrowed face".into());
    };
    let built = profile::build_profile(model, prof)?;
    let mut other = crate::chain::absorb_shape(model, shape)?;
    if let Some(matrix) = transform {
        other = super::pattern::moved(model, &other, matrix)?;
    }
    let at = point3(point);
    let face = face_at(model, &other, at)?.ok_or_else(|| {
        format!(
            "no face of the borrowed shape lies at ({:.1}, {:.1}, {:.1}), where the target face \
             was picked",
            at.x, at.y, at.z
        )
    })?;
    let met = nearest_point_on(model, &face, at).unwrap_or(at);
    revolve(
        model,
        None,
        prof,
        &built,
        *axis_origin,
        *axis_dir,
        *angle_deg,
        *second_angle_deg,
        *midplane,
        *reversed,
        Stop::Face(face, met),
    )
}

/// The tool a flat face of `base` sweeps: the face, copied out of the
/// solid with its outer and inner boundaries, extruded as a sketch on its
/// plane with its outward normal would be.
pub fn build_face_tool(
    model: &mut Model,
    base: Option<&Shape>,
    face: &FaceProbe,
    kind: &SweepKind,
) -> Result<Shape, String> {
    let base = base.ok_or("extruding a face needs a solid to take the face from")?;
    if !matches!(kind, SweepKind::Extrude { .. }) {
        return Err("a face of the solid is only ever extruded".into());
    }
    let found = face_by_name(model, base, face.name, point3(face.point))?
        .ok_or("no face of the solid lies where the face was picked")?;
    face_tool(model, Some(base), &found, kind)
}

/// Extrude the face at `point` of `shape` (native format, moved by
/// `transform`): a face another body lends, as a pad's or pocket's profile.
pub fn build_face_of_tool(
    model: &mut Model,
    base: Option<&Shape>,
    shape: &[u8],
    transform: Option<&[[f64; 4]; 4]>,
    point: [f64; 3],
    kind: &SweepKind,
) -> Result<Shape, String> {
    if !matches!(kind, SweepKind::Extrude { .. }) {
        return Err("a borrowed face is only ever extruded".into());
    }
    let mut other = crate::chain::absorb_shape(model, shape)?;
    if let Some(matrix) = transform {
        other = super::pattern::moved(model, &other, matrix)?;
    }
    let at = point3(point);
    let found = face_at(model, &other, at)?.ok_or_else(|| {
        format!(
            "no face of the borrowed shape lies at ({:.1}, {:.1}, {:.1}), where it was picked",
            at.x, at.y, at.z
        )
    })?;
    face_tool(model, base, &found, kind)
}

/// Extrude `found`, a flat face, out of the solid it faces out of.
fn face_tool(
    model: &mut Model,
    base: Option<&Shape>,
    found: &Shape,
    kind: &SweepKind,
) -> Result<Shape, String> {
    let (_, normal) = face_plane(model, found).ok_or("the picked face is not flat")?;
    let outward = if found.orientation() == ogeom::topo::Orientation::Reversed {
        normal.reversed()
    } else {
        normal
    };
    let copy = ogeom::algo::copied(model, found)
        .map_err(|e| format!("copying the picked face failed: {e}"))?
        .shape;
    let built = BuiltProfile {
        faces: vec![copy],
        groups: Vec::new(),
    };
    extrude(model, base, &built, outward, kind)
}

fn extrude(
    model: &mut Model,
    base: Option<&Shape>,
    built: &BuiltProfile,
    normal: Direction,
    kind: &SweepKind,
) -> Result<Shape, String> {
    let SweepKind::Extrude {
        termination,
        second_side,
        symmetric,
        reversed,
        taper_deg,
        direction,
    } = kind
    else {
        return Err("not an extrusion".into());
    };
    let (symmetric, taper_deg) = (*symmetric, *taper_deg);
    let second_side = second_side.as_ref();
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
    if *reversed {
        dir = dir.reversed();
    }

    // Two plain lengths, or one centred on the sketch plane, make one
    // prism from its far end: no seam on the sketch plane, where a sketch
    // on a face of the solid would put an edge lying in that face, which
    // the boolean does not resolve; and no solid moved after it is built,
    // which a cut through that face refuses as well.
    if let (
        ExtrudeTermination::Blind { distance: front },
        Some(ExtrudeTermination::Blind { distance: back }),
    ) = (termination, second_side)
        && taper_deg.abs() <= 1e-12
        && *front + *back > 0.0
    {
        return one_prism_from(model, built, dir, -*back, *front + *back);
    }
    if let ExtrudeTermination::Blind { distance } = termination
        && symmetric
        && second_side.is_none()
        && taper_deg.abs() <= 1e-12
        && *distance > 0.0
    {
        return one_prism_from(model, built, dir, -distance * 0.5, *distance);
    }
    let mut tool = extrude_one_side(model, base, built, dir, termination, taper_deg)?;
    if let Some(term2) = second_side {
        let back = extrude_one_side(model, base, built, dir.reversed(), term2, taper_deg)?;
        tool = ogeom::boolean::fuse(model, &tool, &back, tol())
            .map_err(|e| format!("fusing the two sweep sides failed: {e}"))?
            .shape;
    } else if symmetric && let ExtrudeTermination::Blind { distance } = termination {
        let shift = Transform::translation(dir.vector() * (-distance * 0.5));
        tool = ogeom::algo::transformed(model, &tool, shift)
            .map_err(|e| format!("centering the symmetric extrusion failed: {e}"))?
            .shape;
    }
    Ok(tool)
}

/// The profile's faces moved `start` along `dir` and extruded `length`
/// along it, fused into one tool.
fn one_prism_from(
    model: &mut Model,
    built: &BuiltProfile,
    dir: Direction,
    start: f64,
    length: f64,
) -> Result<Shape, String> {
    let shift = Transform::translation(dir.vector() * start);
    let mut parts = Vec::with_capacity(built.faces.len());
    for face in &built.faces {
        let moved = ogeom::algo::transformed(model, face, shift)
            .map_err(|e| format!("placing the extrusion's start failed: {e}"))?
            .shape;
        let part = make_prism(model, &moved, dir.vector() * length, tol())
            .map_err(|e| format!("extrude operation failed: {e}"))?;
        parts.push(part.shape);
    }
    fuse_all(model, parts)
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
        ExtrudeTermination::UpToFace {
            point,
            normal,
            offset,
            name,
        } => {
            let at = Point::new(point[0], point[1], point[2]);
            let target = match base {
                Some(base) => match crate::naming::find_face(model, base, *name, at) {
                    Some(face) => Some(face),
                    None => face_at(model, base, at)?,
                },
                None => None,
            };
            match (base, target) {
                (Some(base), Some(face)) => {
                    up_to_face(model, built, base, dir, &face, *offset, taper_deg)
                }
                _ => {
                    let plane_normal =
                        Direction::new(Vector::new(normal[0], normal[1], normal[2]), tol())
                            .map_err(|_| "target face normal is (near) zero".to_string())?;
                    let plane_point = at + plane_normal.vector() * *offset;
                    up_to_plane(model, built, dir, plane_point, plane_normal, taper_deg)
                }
            }
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
                None => up_to_face(model, built, base, dir, &hit.face, 0.0, taper_deg),
            }
        }
        ExtrudeTermination::UpToShape { faces, offset } => {
            let base = base.ok_or_else(|| {
                "an up-to-shape extrusion needs existing material to stop at".to_string()
            })?;
            let stops = base_stop_faces(model, base, faces)?;
            up_to_shape(model, built, Some(base), dir, stops, *offset, taper_deg)
        }
        ExtrudeTermination::UpToShapeOf {
            faces,
            others,
            offset,
        } => {
            let mut stops = match base {
                Some(base) => base_stop_faces(model, base, faces)?,
                None if faces.is_empty() => Vec::new(),
                None => {
                    return Err(
                        "stopping on the solid's own faces needs existing material".to_string()
                    );
                }
            };
            for other in others {
                let mut shape = crate::chain::absorb_shape(model, &other.shape)?;
                if let Some(matrix) = &other.transform {
                    shape = super::pattern::moved(model, &shape, matrix)?;
                }
                for point in &other.points {
                    let at = point3(*point);
                    let face = face_at(model, &shape, at)?.ok_or_else(|| {
                        format!(
                            "no face of the borrowed shape lies at ({:.1}, {:.1}, {:.1}), \
                             where a stop face was picked",
                            at.x, at.y, at.z
                        )
                    })?;
                    stops.push((face, at));
                }
            }
            up_to_shape(model, built, base, dir, stops, *offset, taper_deg)
        }
        ExtrudeTermination::UpToFaceOf {
            shape,
            transform,
            point,
            offset,
        } => {
            let mut other = crate::chain::absorb_shape(model, shape)?;
            if let Some(matrix) = transform {
                other = super::pattern::moved(model, &other, matrix)?;
            }
            let at = point3(*point);
            let face = face_at(model, &other, at)?.ok_or_else(|| {
                format!(
                    "no face of the borrowed shape lies at ({:.1}, {:.1}, {:.1}), where the \
                     target face was picked",
                    at.x, at.y, at.z
                )
            })?;
            up_to_face(model, built, &other, dir, &face, *offset, taper_deg)
        }
    }
}

/// The faces of `base` the probes name, each with where it was picked.
fn base_stop_faces(
    model: &mut Model,
    base: &Shape,
    faces: &[FaceProbe],
) -> Result<Vec<(Shape, Point)>, String> {
    faces
        .iter()
        .map(|probe| {
            let at = point3(probe.point);
            let face = face_by_name(model, base, probe.name, at)?.ok_or_else(|| {
                format!(
                    "no face of the solid lies at ({:.1}, {:.1}, {:.1}), where a stop face was \
                     picked",
                    at.x, at.y, at.z
                )
            })?;
            Ok((face, at))
        })
        .collect()
}

/// A prism from the profile in which every line of the sweep ends on the
/// first of `stops` it meets: a prism long enough to pass them all (and
/// the base solid, when there is one), less each face's shadow (the face
/// swept on along the sweep), kept where it starts at the profile.
fn up_to_shape(
    model: &mut Model,
    built: &BuiltProfile,
    base: Option<&Shape>,
    dir: Direction,
    stops: Vec<(Shape, Point)>,
    offset: f64,
    taper_deg: f64,
) -> Result<Shape, String> {
    if stops.is_empty() {
        return Err("pick the faces the extrusion stops at".into());
    }
    let centroid = profile::profile_centroid(model, built)?;
    let mut reach = 0.0f64;
    for shape in base.into_iter().chain(stops.iter().map(|(face, _)| face)) {
        reach = reach.max(through_all_length(model, shape, centroid, dir)?);
    }
    let reach = reach + offset.abs();
    let mut tool = prism_solid(model, built, dir, reach, taper_deg)?;
    for (face, at) in &stops {
        // A flat face the sweep runs along stops none of it.
        if face_plane(model, face).is_some_and(|(_, n)| n.dot(dir).abs() <= 1e-9) {
            continue;
        }
        let ahead = tess::robust_bounds(model, face).is_some_and(|(lo, hi)| {
            [lo.x, hi.x].iter().any(|&x| {
                [lo.y, hi.y].iter().any(|&y| {
                    [lo.z, hi.z]
                        .iter()
                        .any(|&z| (Point::new(x, y, z) - centroid).dot(dir.vector()) > 1e-6)
                })
            })
        });
        if !ahead {
            return Err(format!(
                "the stop face at ({:.1}, {:.1}, {:.1}) lies behind the profile",
                at.x, at.y, at.z
            ));
        }
        let mut stop = ogeom::algo::copied(model, face)
            .map_err(|e| format!("copying a stop face failed: {e}"))?
            .shape;
        if offset != 0.0 {
            let shift = Transform::translation(dir.vector() * offset);
            stop = ogeom::algo::transformed(model, &stop, shift)
                .map_err(|e| format!("moving a stop face by its offset failed: {e}"))?
                .shape;
        }
        let shadow = make_prism(model, &stop, dir.vector() * (2.0 * reach), tol())
            .map_err(|e| format!("sweeping a stop face on failed: {e}"))?
            .shape;
        tool = ogeom::boolean::cut(model, &tool, &shadow, tol())
            .map_err(|e| format!("stopping the sweep at a stop face failed: {e}"))?
            .shape;
        tool = super::normalized(model, tool);
    }
    starting_at_profile(model, tool, centroid, dir)
}

/// The pieces of `trimmed` the profile starts: a curved stop can leave a
/// long prism coming back out beyond it.
fn starting_at_profile(
    model: &mut Model,
    trimmed: Shape,
    centroid: Point,
    dir: Direction,
) -> Result<Shape, String> {
    let trimmed = super::normalized(model, trimmed);
    let starting: Vec<Shape> = super::solids_of(model, &trimmed)
        .into_iter()
        .filter(|piece| {
            tess::robust_bounds(model, piece).is_some_and(|(lo, hi)| {
                // The piece's nearest corner along the sweep: at the profile
                // for a piece the profile starts.
                let mut nearest = f64::MAX;
                for &x in &[lo.x, hi.x] {
                    for &y in &[lo.y, hi.y] {
                        for &z in &[lo.z, hi.z] {
                            nearest =
                                nearest.min((Point::new(x, y, z) - centroid).dot(dir.vector()));
                        }
                    }
                }
                nearest <= 1e-3
            })
        })
        .collect();
    super::wrap_pieces(model, starting)
        .map_err(|_| "the sweep does not reach the target face".to_string())
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

/// How far from a picked point the face it names may be: a pick lands on
/// a face's drawn triangles, a chord off a curved surface.
const FACE_REACH_MM: f64 = 0.5;

/// The face of `base` named `name`, else the one [`face_at`] finds.
pub(crate) fn face_by_name(
    model: &mut Model,
    base: &Shape,
    name: kernel_api::TopoName,
    point: Point,
) -> Result<Option<Shape>, String> {
    match crate::naming::find_face(model, base, name, point) {
        Some(face) => Ok(Some(face)),
        None => face_at(model, base, point),
    }
}

/// The point of `face` nearest `point`.
fn nearest_point_on(model: &mut Model, face: &Shape, point: Point) -> Option<Point> {
    let probe = model.add_vertex(ogeom::topo::VertexData::new(point));
    ogeom::algo::distance_between_shapes(
        model,
        &probe,
        face,
        ogeom::intersect::ExtremaOptions::default(),
        tol(),
    )
    .ok()?
    .pairs
    .first()
    .map(|pair| pair.point_b)
}

/// The face of `base` nearest `point`, when one comes within reach of it.
pub(crate) fn face_at(
    model: &mut Model,
    base: &Shape,
    point: Point,
) -> Result<Option<Shape>, String> {
    let face = super::dressup::nearest_of(model, base, ShapeType::Face, point)?;
    let probe = model.add_vertex(ogeom::topo::VertexData::new(point));
    let near = ogeom::algo::distance_between_shapes(
        model,
        &probe,
        &face,
        ogeom::intersect::ExtremaOptions::default(),
        tol(),
    )
    .map_err(|e| format!("measuring to the target face failed: {e}"))?;
    Ok((near.distance <= FACE_REACH_MM).then_some(face))
}

/// A face's whole surface, where the face sits, as a face of its own:
/// what a half-space needs to trim by the surface beyond the face's edges.
/// `offset` pushes it out along the face's outward normal.
fn whole_surface(model: &mut Model, face: &Shape, offset: f64) -> Result<Shape, String> {
    let node = model
        .node(face)
        .ok_or("the target face is not in the model")?;
    let NodeData::Face(data) = node.data() else {
        return Err("the target is not a face".into());
    };
    let surface = model
        .geometry()
        .surface(data.surface)
        .ok_or("the target face has no surface")?
        .clone();
    let placement = face
        .transform(model.datums())
        .map_err(|e| format!("placing the target face failed: {e}"))?;
    let mut surface = surface
        .transformed(&placement, tol())
        .map_err(|e| format!("placing the target surface failed: {e}"))?;
    if offset != 0.0 {
        // The surface's own normal points out of the material unless the
        // face is reversed on it.
        let outward = if face.orientation() == ogeom::topo::Orientation::Reversed {
            -offset
        } else {
            offset
        };
        surface = SurfaceGeometry::Offset(Box::new(
            ogeom::geom::OffsetSurface::new(surface, outward)
                .map_err(|e| format!("offsetting the target face failed: {e}"))?,
        ));
    }
    Ok(make_natural_face(model, surface)
        .map_err(|e| format!("the target surface as a face: {e}"))?
        .shape)
}

/// A prism from the profile stopped on `face`'s surface: long enough to
/// pass it, trimmed by the surface's half-space on the profile's side, and
/// kept only where it starts at the profile (a curved surface can let the
/// long prism back out beyond it).
fn up_to_face(
    model: &mut Model,
    built: &BuiltProfile,
    base: &Shape,
    dir: Direction,
    face: &Shape,
    offset: f64,
    taper_deg: f64,
) -> Result<Shape, String> {
    if let Some((point, normal)) = face_plane(model, face) {
        let outward = if face.orientation() == ogeom::topo::Orientation::Reversed {
            -normal.vector()
        } else {
            normal.vector()
        };
        let outward = Direction::new(outward, tol()).map_err(|e| e.to_string())?;
        let point = point + outward.vector() * offset;
        return up_to_plane(model, built, dir, point, outward, taper_deg);
    }
    let centroid = profile::profile_centroid(model, built)?;
    let reach = through_all_length(model, base, centroid, dir)? + offset.abs();
    let long_prism = prism_solid(model, built, dir, reach, taper_deg)?;
    let surface = whole_surface(model, face, offset)?;
    let half = ogeom::algo::make_half_space(model, &surface, centroid, tol())
        .map_err(|e| format!("the target surface's half-space: {e}"))?
        .shape;
    let trimmed = ogeom::boolean::common(model, &long_prism, &half, tol())
        .map_err(|e| format!("trimming the sweep at the target face failed: {e}"))?
        .shape;
    starting_at_profile(model, trimmed, centroid, dir)
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
    /// Set when the hit face is planar: its world-space plane.
    pub plane: Option<(Point, Direction)>,
    pub face: Shape,
}

/// Nearest (or farthest) intersection of the ray with the base solid's
/// faces, via each face's triangulation, which is close enough to the exact
/// faces for termination queries.
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
            if let Some(w) = ray_triangle(origin, dir, a, b, c)
                && w > 1e-6
            {
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
    let Some((_, face_idx)) = best else {
        return Ok(None);
    };
    Ok(Some(RayHit {
        plane: face_plane(model, &faces[face_idx]),
        face: faces[face_idx].clone(),
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
pub(crate) fn ray_triangle(
    origin: Point,
    dir: Direction,
    a: Point,
    b: Point,
    c: Point,
) -> Option<f64> {
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
    base: Option<&Shape>,
    prof: &Profile,
    built: &BuiltProfile,
    axis_origin: [f64; 2],
    axis_dir: [f64; 2],
    angle_deg: f64,
    second_angle_deg: Option<f64>,
    midplane: bool,
    reversed: bool,
    stop: Stop,
) -> Result<Shape, String> {
    let axis = sketch_plane_axis(&prof.plane, axis_origin, axis_dir)?;
    let axis = if reversed { axis_reversed(&axis) } else { axis };

    let (forward, backward) = match stop {
        Stop::Termination(RevolveTermination::Angle) if midplane => {
            (angle_deg * 0.5, angle_deg * 0.5)
        }
        Stop::Termination(RevolveTermination::Angle) => {
            (angle_deg, second_angle_deg.unwrap_or(0.0))
        }
        stop => {
            let (face, met_at) = match stop {
                Stop::Face(face, met_at) => (face, met_at),
                Stop::Termination(termination) => {
                    let base = base.ok_or_else(|| {
                        "a revolution that stops on a face needs existing material".to_string()
                    })?;
                    revolve_target(model, base, built, &axis, termination)?
                }
            };
            match revolve_stop(model, built, &axis, face, met_at)? {
                RevolveStop::Angle(angle) => (angle.to_degrees(), 0.0),
                RevolveStop::Face(limit) => {
                    // Each point turns until its own circle meets the face.
                    let mut parts = Vec::with_capacity(built.faces.len());
                    for face in &built.faces {
                        let part = make_revolution_until(model, face, axis, &limit, tol())
                            .map_err(|e| format!("revolve up to the face failed: {e}"))?;
                        parts.push(part.shape);
                    }
                    return fuse_all(model, parts);
                }
            }
        }
    };
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
    // plane makes the seam edge coincide with any base face on that plane:
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

/// Where a revolution that stops on a face stops.
enum RevolveStop {
    /// A flat face whose plane holds the axis: every point of the profile
    /// meets it at this angle, in radians.
    Angle(f64),
    /// Any other face: each point meets it at an angle of its own.
    Face(Shape),
}

/// What a revolution turns until: its termination, or a face of another
/// shape and where the turn meets it.
enum Stop<'a> {
    Termination(&'a RevolveTermination),
    Face(Shape, Point),
}

/// The face of `base` the termination `stop` names, and where the turn
/// meets it.
fn revolve_target(
    model: &mut Model,
    base: &Shape,
    built: &BuiltProfile,
    axis: &Axis,
    stop: &RevolveTermination,
) -> Result<(Shape, Point), String> {
    let centroid = profile::profile_centroid(model, built)?;
    Ok(match stop {
        RevolveTermination::UpToFace(probe) => {
            let at = point3(probe.point);
            let face = face_by_name(model, base, probe.name, at)?
                .ok_or("no face of the solid lies where the target face was picked")?;
            // The face may have moved since it was picked: where the turn
            // meets it is on it.
            let met = nearest_point_on(model, &face, at).unwrap_or(at);
            (face, met)
        }
        RevolveTermination::ToFirst | RevolveTermination::ToLast => {
            let first = matches!(stop, RevolveTermination::ToFirst);
            let hit = circle_hit(model, base, axis, centroid, first)?
                .ok_or("turning, the profile meets no face of the existing material")?;
            (hit.face, hit.point)
        }
        RevolveTermination::Angle => return Err("a revolution by its angle has no stop".into()),
    })
}

/// Where the profile turning about `axis` stops on `face`, met at
/// `met_at`.
fn revolve_stop(
    model: &mut Model,
    built: &BuiltProfile,
    axis: &Axis,
    face: Shape,
    met_at: Point,
) -> Result<RevolveStop, String> {
    let a = axis.direction.vector();
    let from_axis = |p: Point| {
        let v = p - axis.location;
        v - a * v.dot(a)
    };
    let centroid = profile::profile_centroid(model, built)?;
    let start = from_axis(centroid);
    if start.magnitude() <= 1e-9 {
        return Err("the profile's centre lies on the revolution axis".into());
    }
    let holds_axis = face_plane(model, &face).is_some_and(|(p, n)| {
        n.dot(axis.direction).abs() <= 1e-6 && (axis.location - p).dot(n.vector()).abs() <= 1e-4
    });
    if !holds_axis {
        return Ok(RevolveStop::Face(face));
    }
    let end = from_axis(met_at);
    if end.magnitude() <= 1e-9 {
        return Err("the target face is met on the revolution axis".into());
    }
    let mut angle = start.cross(end).dot(a).atan2(start.dot(end));
    if angle <= 1e-9 {
        angle += TAU;
    }
    if angle >= TAU - 1e-9 {
        return Err("the profile already lies on the target face".into());
    }
    Ok(RevolveStop::Angle(angle))
}

/// Where a point turning about an axis first meets a face.
struct CircleHit {
    face: Shape,
    point: Point,
}

/// The first (or last) face of `base` the circle `from` runs about `axis`
/// meets, turning the right-handed way about it, via each face's
/// triangulation.
fn circle_hit(
    model: &Model,
    base: &Shape,
    axis: &Axis,
    from: Point,
    first: bool,
) -> Result<Option<CircleHit>, String> {
    let a = axis.direction.vector();
    let centre = axis.location + a * (from - axis.location).dot(a);
    let radial = from - centre;
    let radius = radial.magnitude();
    if radius <= 1e-9 {
        return Ok(None);
    }
    let u = radial * (1.0 / radius);
    let w = a.cross(u);
    let on_circle = |angle: f64| centre + u * (radius * angle.cos()) + w * (radius * angle.sin());
    let faces = explore(model, base, Filter::OfType(ShapeType::Face))
        .map_err(|e| format!("exploring base faces: {e}"))?;
    let mut best: Option<(f64, usize, Point)> = None;
    for (i, face) in faces.iter().enumerate() {
        let Ok(tri) = triangulate_face(model, face, Deflection::default(), tol()) else {
            continue;
        };
        for t in &tri.triangles {
            let [p0, p1, p2] = t.map(|i| tri.positions[i as usize]);
            let n = (p1 - p0).cross(p2 - p0);
            if n.magnitude() <= 1e-12 {
                continue;
            }
            // n · (circle(θ) − p0) = 0 is A cos θ + B sin θ = D.
            let (ca, cb) = (radius * n.dot(u), radius * n.dot(w));
            let d = n.dot(p0 - centre);
            let m = ca.hypot(cb);
            if m <= 1e-12 || d.abs() > m {
                continue;
            }
            let phase = cb.atan2(ca);
            let spread = (d / m).clamp(-1.0, 1.0).acos();
            for angle in [phase - spread, phase + spread] {
                let angle = angle.rem_euclid(TAU);
                if !(1e-6..=TAU - 1e-6).contains(&angle) {
                    continue;
                }
                let q = on_circle(angle);
                if !in_triangle(q, p0, p1, p2, n) {
                    continue;
                }
                let better = best.is_none_or(|(b, ..)| if first { angle < b } else { angle > b });
                if better {
                    best = Some((angle, i, q));
                }
            }
        }
    }
    Ok(best.map(|(_, i, point)| CircleHit {
        face: faces[i].clone(),
        point,
    }))
}

/// Whether `q`, on the plane of triangle `a b c` (normal `n`), lies in it.
fn in_triangle(q: Point, a: Point, b: Point, c: Point, n: Vector) -> bool {
    let inside = |p: Point, s: Point| (s - p).cross(q - p).dot(n) >= -1e-9 * n.dot(n);
    inside(a, b) && inside(b, c) && inside(c, a)
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

/// How far a helix runs: its advance per turn, how many turns, and how far
/// every point moves off the axis per turn.
#[derive(Debug, Clone, Copy, PartialEq)]
struct HelixExtent {
    pitch: f64,
    turns: f64,
    growth: f64,
}

impl HelixExtent {
    /// The extent a helix's numbers give. A given turn count sets the pitch
    /// from the height; a height of 0 is a flat spiral, which must grow.
    fn of(
        pitch: f64,
        height: f64,
        turns: Option<f64>,
        cone_angle_deg: f64,
        growth: Option<f64>,
    ) -> Result<Self, String> {
        let (pitch, turns) = match turns {
            Some(turns) => {
                if !(turns.is_finite() && turns > 1e-9) {
                    return Err("helix turns must be positive".into());
                }
                if !(height.is_finite() && height >= 0.0) {
                    return Err("helix height must not be negative".into());
                }
                (height / turns, turns)
            }
            None => {
                if pitch <= 1e-9 || height <= 1e-9 {
                    return Err("helix pitch and height must be positive".into());
                }
                (pitch, height / pitch)
            }
        };
        // A cone moves each point off the axis by tan(angle) of every
        // pitch it climbs.
        let growth = growth.unwrap_or_else(|| pitch * cone_angle_deg.to_radians().tan());
        if !growth.is_finite() {
            return Err("helix growth per turn is not a length".into());
        }
        if pitch <= 1e-9 && growth.abs() <= 1e-9 {
            return Err("a flat spiral needs a growth per turn".into());
        }
        Ok(Self {
            pitch,
            turns,
            growth,
        })
    }
}

#[expect(clippy::too_many_arguments)]
fn helix(
    model: &mut Model,
    prof: &Profile,
    built: &BuiltProfile,
    axis_origin: [f64; 2],
    axis_dir: [f64; 2],
    extent: HelixExtent,
    left_handed: bool,
    reversed: bool,
) -> Result<Shape, String> {
    let axis = sketch_plane_axis(&prof.plane, axis_origin, axis_dir)?;
    helix_about(model, built, axis, extent, left_handed, reversed)
}

/// The helical sweep of the profile's faces about `axis`, which the
/// profile must not sit on.
fn helix_about(
    model: &mut Model,
    built: &BuiltProfile,
    axis: Axis,
    extent: HelixExtent,
    left_handed: bool,
    reversed: bool,
) -> Result<Shape, String> {
    let centroid = profile::profile_centroid(model, built)?;
    let to_start = centroid - axis.location;
    let along = axis.direction.vector() * to_start.dot(axis.direction.vector());
    if (to_start - along).magnitude() <= 1e-9 {
        return Err("the profile sits on the helix axis (zero radius)".into());
    }
    let direction = if reversed {
        axis.direction.reversed()
    } else {
        axis.direction
    };
    let axis = Axis::new(axis.location, direction);
    // Every point of the profile runs its own helix about the axis.
    let mut parts = Vec::with_capacity(built.faces.len());
    for face in &built.faces {
        let part = ogeom::offset::make_helical_sweep(
            model,
            face,
            axis,
            extent.pitch,
            extent.turns,
            left_handed,
            extent.growth,
            tol(),
        )
        .map_err(|e| format!("helix sweep failed: {e}"))?;
        parts.push(part.shape);
    }
    fuse_all(model, parts)
}

#[cfg(test)]
mod tests {
    use super::HelixExtent;

    #[test]
    fn a_helix_extent_reads_its_numbers_the_way_they_were_given() {
        let by_pitch = HelixExtent::of(2.0, 10.0, None, 0.0, None).unwrap();
        assert_eq!((by_pitch.pitch, by_pitch.turns), (2.0, 5.0));
        assert!(by_pitch.growth.abs() < 1e-12);

        let by_turns = HelixExtent::of(99.0, 12.0, Some(4.0), 0.0, Some(1.5)).unwrap();
        assert_eq!(
            (by_turns.pitch, by_turns.turns, by_turns.growth),
            (3.0, 4.0, 1.5)
        );

        let cone = HelixExtent::of(2.0, 10.0, None, 45.0, None).unwrap();
        assert!((cone.growth - 2.0).abs() < 1e-12);
    }

    #[test]
    fn a_flat_spiral_has_no_pitch_and_must_grow() {
        let flat = HelixExtent::of(0.0, 0.0, Some(3.0), 0.0, Some(2.0)).unwrap();
        assert_eq!((flat.pitch, flat.turns, flat.growth), (0.0, 3.0, 2.0));
        assert!(HelixExtent::of(0.0, 0.0, Some(3.0), 0.0, None).is_err());
        assert!(HelixExtent::of(0.0, 0.0, Some(3.0), 0.0, Some(0.0)).is_err());
        assert!(HelixExtent::of(2.0, 0.0, None, 0.0, None).is_err());
    }
}
