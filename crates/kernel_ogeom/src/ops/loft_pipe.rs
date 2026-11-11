//! Loft through section profiles and sweep along a sketch spine.

use kernel_api::{LoftSection, PipeCorner, PipeFrame, PipePath, Profile};
use ogeom::algo::transformed;
use ogeom::geom::Curve3d;
use ogeom::math::{Axis, Direction, Point, Transform, Vector};
use ogeom::offset::{
    PipeCorners, PipeLaw, make_loft, make_loft_skinned, make_loft_skinned_closed,
    make_pipe_sections, make_pipe_shell_with,
};
use ogeom::topo::{Model, NodeData, Shape};

use super::{fuse_all, tol};
use crate::convert::{point, vector};
use crate::profile;

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
    loft_sections(model, built, ruled, closed)
}

/// A loft through sections that may be profiles, flat faces of `base` or
/// points (the first or the last).
pub fn loft_through_tool(
    model: &mut Model,
    base: Option<&Shape>,
    sections: &[LoftSection],
    ruled: bool,
    closed: bool,
) -> Result<Shape, String> {
    if sections.len() < 2 {
        return Err("loft needs at least two sections".into());
    }
    let last = sections.len() - 1;
    let mut built = Vec::with_capacity(sections.len());
    for (i, section) in sections.iter().enumerate() {
        built.push(match section {
            LoftSection::Profile(p) => single_region(model, p, "a loft section")?,
            LoftSection::Face(probe) => face_section(model, base, probe)?,
            LoftSection::Point(p) => {
                if i != 0 && i != last {
                    return Err(
                        "only the first or the last section of a loft may be a point".into(),
                    );
                }
                if closed {
                    return Err("a closed loop of sections takes no point".into());
                }
                Section {
                    outer: model.add_vertex(ogeom::topo::VertexData::new(point(p))),
                    holes: Vec::new(),
                }
            }
        });
    }
    // The kernel closes to a point at the end: a first point goes last.
    if matches!(sections[0], LoftSection::Point(_)) {
        built.reverse();
    }
    loft_sections(model, built, ruled, closed)
}

/// The boundaries of the flat face of `base` the probe finds.
fn face_section(
    model: &mut Model,
    base: Option<&Shape>,
    probe: &kernel_api::FaceProbe,
) -> Result<Section, String> {
    let base = base.ok_or("a face section needs a solid to take the face from")?;
    let face = super::sweep::face_by_name(model, base, probe.name, point(probe.point))?
        .ok_or("no face of the solid lies where the section's face was picked")?;
    let mut wires: Vec<(f64, Shape)> = model
        .children_of(&face)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|c| model.kind_of(c) == Ok(ogeom::topo::ShapeType::Wire))
        .map(|w| {
            let size = crate::tess::robust_bounds(model, &w)
                .map(|(lo, hi)| (hi - lo).magnitude())
                .unwrap_or(0.0);
            (size, w)
        })
        .collect();
    wires.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut wires = wires.into_iter().map(|(_, w)| w);
    let outer = wires.next().ok_or("the section's face has no boundary")?;
    Ok(Section {
        outer,
        holes: wires.collect(),
    })
}

fn loft_sections(
    model: &mut Model,
    built: Vec<Section>,
    ruled: bool,
    closed: bool,
) -> Result<Shape, String> {
    let points = built
        .iter()
        .filter(|s| model.kind_of(&s.outer) == Ok(ogeom::topo::ShapeType::Vertex))
        .count();
    if points > 0 && built.iter().any(|s| !s.holes.is_empty()) {
        return Err("a loft to a point takes sections without holes".into());
    }
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
    // Two sections, one a point: the smooth loft is the ruled one, a cone
    // or a pyramid, built exactly.
    let to_point = wires
        .iter()
        .any(|w| model.kind_of(w) == Ok(ogeom::topo::ShapeType::Vertex));
    if ruled || (to_point && wires.len() == 2) {
        // Chain of two-section ruled lofts, fused. The ruled construction
        // takes circles on one axis and polygons of one corner count;
        // between other sections (a square and a circle) the skinned loft,
        // which through two sections is the ruled one, builds the pair.
        let mut parts = Vec::with_capacity(wires.len() - 1);
        for pair in wires.windows(2) {
            let part = match make_loft(model, &pair[0], &pair[1], tol()) {
                Ok(part) => part,
                Err(ruled) => make_loft_skinned(model, pair, SKIN_TOLERANCE, tol())
                    .map_err(|e| format!("ruled loft failed: {ruled}; skinned: {e}"))?,
            };
            parts.push(part.shape);
        }
        fuse_all(model, parts)
    } else {
        make_loft_skinned(model, wires, SKIN_TOLERANCE, tol())
            .map(|b| b.shape)
            .map_err(|e| format!("loft failed: {e}"))
    }
}

/// Sweep `prof` down `spine`, the section turning as `frame` says and
/// turning the path's sharp corners as `corner` says; with further
/// `sections`, the section changes shape down the path through each, where
/// the path crosses its plane.
pub fn pipe_tool(
    model: &mut Model,
    prof: &Profile,
    spine: &Profile,
    frame: &PipeFrame,
    corner: PipeCorner,
    sections: &[Profile],
) -> Result<Shape, String> {
    let built = profile::build_profile(model, prof)?;
    let normal = profile::plane_normal(&prof.plane).map_err(|e| format!("pipe profile: {e}"))?;
    let spine_wire = spine_of(model, spine)?;
    let mut faces = Vec::with_capacity(sections.len());
    for section in sections {
        faces.push(section_face(model, section)?);
    }
    pipe_from(model, built, normal, spine_wire, frame, corner, faces)
}

/// The wire of a sketch's path.
fn spine_of(model: &mut Model, spine: &Profile) -> Result<Shape, String> {
    let spine_wire_desc = spine
        .wires
        .first()
        .ok_or_else(|| "pipe spine has no wire".to_string())?;
    let spine_built = profile::build_wire_edges(model, &spine.plane, spine_wire_desc, false)
        .map_err(|e| format!("pipe spine: {e}"))?;
    Ok(spine_built.wire.clone())
}

/// The single region a section profile encloses, as a face.
fn section_face(model: &mut Model, section: &Profile) -> Result<Shape, String> {
    let section = profile::build_profile(model, section)?;
    let [face] = section.faces.as_slice() else {
        return Err("each of the pipe's sections must enclose a single region".into());
    };
    Ok(face.clone())
}

/// A pipe whose profile may be a face of `base`, whose path may be edges
/// of a solid, and whose sections may end at a point.
#[allow(clippy::too_many_arguments)]
pub fn pipe_through_tool(
    model: &mut Model,
    base: Option<&Shape>,
    profile: &LoftSection,
    path: &PipePath,
    frame: &PipeFrame,
    corner: PipeCorner,
    sections: &[LoftSection],
) -> Result<Shape, String> {
    let (built, normal) = match profile {
        LoftSection::Profile(prof) => (
            profile::build_profile(model, prof)?,
            profile::plane_normal(&prof.plane).map_err(|e| format!("pipe profile: {e}"))?,
        ),
        LoftSection::Face(probe) => {
            let base = base.ok_or("a face as the profile needs a solid to take it from")?;
            let found = super::sweep::face_by_name(model, base, probe.name, point(probe.point))?
                .ok_or("no face of the solid lies where the profile's face was picked")?;
            let (_, normal) =
                super::sweep::face_plane(model, &found).ok_or("the profile's face is not flat")?;
            let copy = ogeom::algo::copied(model, &found)
                .map_err(|e| format!("copying the profile's face failed: {e}"))?
                .shape;
            (
                profile::BuiltProfile {
                    faces: vec![copy],
                    groups: Vec::new(),
                },
                normal,
            )
        }
        LoftSection::Point(_) => return Err("a pipe's profile is a region, not a point".into()),
    };
    let spine_wire = match path {
        PipePath::Profile(spine) => spine_of(model, spine)?,
        PipePath::Edges(picks) => {
            let base = base.ok_or("a path along edges needs a solid to take them from")?;
            let edges = super::dressup::picked_edges(model, base, picks)?;
            edge_wire(model, &edges)?
        }
        PipePath::EdgesOf {
            shape,
            transform,
            edges,
        } => {
            let mut other = crate::chain::absorb_shape(model, shape)?;
            if let Some(matrix) = transform {
                other = super::pattern::moved(model, &other, matrix)?;
            }
            let found = super::dressup::picked_edges_elsewhere(model, &other, edges)?;
            edge_wire(model, &found)?
        }
    };
    let last = sections.len().saturating_sub(1);
    let mut faces = Vec::with_capacity(sections.len());
    for (i, section) in sections.iter().enumerate() {
        faces.push(match section {
            LoftSection::Profile(p) => section_face(model, p)?,
            LoftSection::Point(p) if i == last => {
                model.add_vertex(ogeom::topo::VertexData::new(point(p)))
            }
            LoftSection::Point(_) => {
                return Err("only the last of a pipe's sections may be a point".into());
            }
            LoftSection::Face(_) => {
                return Err("a pipe's further sections are sketches or a last point".into());
            }
        });
    }
    pipe_from(model, built, normal, spine_wire, frame, corner, faces)
}

/// Edges joined end to end into one wire, in whatever order they come.
fn edge_wire(model: &mut Model, edges: &[Shape]) -> Result<Shape, String> {
    if edges.is_empty() {
        return Err("the pipe's path names no edges".into());
    }
    ogeom::algo::make_wire_unordered(model, edges, tol())
        .map(|b| b.shape)
        .map_err(|e| format!("the path's edges do not join end to end: {e}"))
}

/// Sweep the built profile (whose plane's normal is `normal`) down
/// `spine_wire`, as [`pipe_tool`] says.
fn pipe_from(
    model: &mut Model,
    built: profile::BuiltProfile,
    normal: Direction,
    spine_wire: Shape,
    frame: &PipeFrame,
    corner: PipeCorner,
    section_faces: Vec<Shape>,
) -> Result<Shape, String> {
    let guide = match frame {
        PipeFrame::Auxiliary { path } => {
            let wire = path
                .wires
                .first()
                .ok_or_else(|| "the pipe's auxiliary path has no wire".to_string())?;
            Some(
                profile::build_wire(model, &path.plane, wire, false)
                    .map_err(|e| format!("the pipe's auxiliary path: {e}"))?,
            )
        }
        _ => None,
    };
    let law = match frame {
        PipeFrame::RotationMinimizing => PipeLaw::RotationMinimizing,
        PipeFrame::Frenet => PipeLaw::Frenet,
        PipeFrame::Binormal { direction } => {
            let d = vector(direction);
            PipeLaw::Binormal(
                Direction::new(d, tol())
                    .map_err(|_| "the pipe's binormal direction is zero".to_string())?,
            )
        }
        PipeFrame::Auxiliary { .. } => PipeLaw::Auxiliary {
            guide: guide.as_ref().expect("built above"),
        },
        PipeFrame::Fixed => PipeLaw::Fixed,
    };
    let corners = match corner {
        PipeCorner::Transformed => PipeCorners::Mitre,
        PipeCorner::Right => PipeCorners::Extended,
        PipeCorner::Round => PipeCorners::Round,
    };

    let (start, tangent) = spine_start(model, &spine_wire)?;

    // The sweep wants the profile sitting at the spine start, square to its
    // tangent. Sketches rarely oblige exactly: rotate the profile's normal
    // onto the tangent and move its centroid onto the start.
    let centroid = profile::profile_centroid(model, &built)?;
    let rotate = rotation_aligning(normal, tangent, centroid);
    let translate = Transform::translation(start - centroid);
    let place = translate * rotate;

    if !section_faces.is_empty() {
        let [face] = built.faces.as_slice() else {
            return Err("a pipe through several sections sweeps a single region".into());
        };
        let mut faces = vec![
            transformed(model, face, place)
                .map_err(|e| format!("placing the pipe profile failed: {e}"))?
                .shape,
        ];
        faces.extend(section_faces);
        let frenet = matches!(frame, PipeFrame::Frenet);
        return make_pipe_sections(model, &faces, &spine_wire, frenet, SKIN_TOLERANCE, tol())
            .map(|b| b.shape)
            .map_err(|e| format!("pipe sweep failed: {e}"));
    }

    let mut parts = Vec::with_capacity(built.faces.len());
    for face in &built.faces {
        let placed = transformed(model, face, place)
            .map_err(|e| format!("placing the pipe profile failed: {e}"))?
            .shape;
        let part = make_pipe_shell_with(
            model,
            &placed,
            &spine_wire,
            &law,
            corners,
            SKIN_TOLERANCE,
            tol(),
        )
        .map_err(|e| format!("pipe sweep failed: {e}"))?;
        parts.push(part.shape);
    }
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
            // The exact tangent: the sweep reads the profile's lean
            // against it, and a chord's direction is off on a curve. A
            // reversed edge starts at its range's end and runs back.
            let reversed = first.orientation() == ogeom::topo::Orientation::Reversed;
            let t0 = if reversed { range.1 } else { range.0 };
            let p0 = geometry
                .point_at(t0, tol())
                .map_err(|e| format!("pipe spine start: {e}"))?;
            let mut d1 = geometry
                .d1_at(t0, tol())
                .map_err(|e| format!("pipe spine tangent: {e}"))?;
            if reversed {
                d1 = -d1;
            }
            let placement = location
                .composed(model.datums())
                .map_err(|e| format!("pipe spine placement: {e}"))?;
            let start = placement.apply(p0);
            let toward = placement.apply(p0 + d1) - start;
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
