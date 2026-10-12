//! Surface steps: sheets made from curves and added to the body beside what
//! is there, and the body's faces sewn, trimmed, split, extended, offset,
//! thickened, rounded (along an edge, or between two faces) or mirrored.
//!
//! A body built from surface steps holds a compound of its sheets (and of
//! the solids sewing or thickening makes). Curves come from sketches (open
//! chains as well as loops), from edges of the body's own shape, or from
//! edges of another body's, carried to where that body sits. Every
//! sheet a step makes is bounded by edges, so the steps after it can pick
//! them and a sew can join it to its neighbours; a fill is bounded by the
//! very edges it was given.

use kernel_api::{Continuity, CurveSource, EdgeProbe, FaceProbe, PipeFrame, SurfaceOp};
use ogeom::algo::build::trimmed_where_bare;
use ogeom::algo::{
    Extension, copied, extend_face, find_plane, make_face, make_prism, make_revolution, make_shell,
    make_solid, make_wire, sew, sew_within, transformed,
};
use ogeom::geom::{PlaneSurface, SurfaceGeometry};
use ogeom::math::{Axis, Direction, Point, Transform, Vector};
use ogeom::offset::{
    FillBoundary, PipeLaw, make_filling_n, make_loft_surface, make_ruled, make_sweep_surface,
    make_sweep_two_rails, make_thick_sheet, offset_sheet,
};
use ogeom::topo::{Model, Shape, ShapeType, explore_unique};

use super::tol;
use crate::profile;

/// What a surface step made: the body's shape after it, and the sheet it
/// added (whose faces are named afresh), when it added one.
pub struct Made {
    pub shape: Shape,
    pub sheet: Option<Shape>,
}

pub fn apply(model: &mut Model, base: Option<&Shape>, op: &SurfaceOp) -> Result<Made, String> {
    if !op.constructs() {
        let base = base.ok_or("there is no surface in the body to work on yet")?;
        let shape = match op {
            SurfaceOp::Sew { gap } => sew_all(model, base, *gap)?,
            SurfaceOp::Mirror { origin, normal } => mirror(model, base, *origin, *normal)?,
            SurfaceOp::Thicken {
                thickness,
                both_sides,
            } => thicken(model, base, *thickness, *both_sides)?,
            SurfaceOp::TrimByPlane { origin, normal } => trim(model, base, *origin, *normal)?,
            SurfaceOp::Split { faces, curves } => split(model, base, faces, curves)?,
            SurfaceOp::Extend {
                edges,
                length,
                continuity,
            } => extend(model, base, edges, *length, *continuity)?,
            SurfaceOp::Fillet { edges, radius } => fillet(model, base, edges, *radius)?,
            SurfaceOp::FilletFaces {
                first,
                second,
                radius,
                flip,
            } => fillet_between(model, base, first, second, *radius, *flip)?,
            _ => unreachable!("constructive steps are handled below"),
        };
        return Ok(Made { shape, sheet: None });
    }
    let sheet = match op {
        SurfaceOp::Extrude {
            curves,
            direction,
            length,
            symmetric,
        } => extrude(model, base, curves, *direction, *length, *symmetric)?,
        SurfaceOp::Revolve {
            curves,
            origin,
            axis,
            angle_deg,
        } => revolve(model, base, curves, *origin, *axis, *angle_deg)?,
        SurfaceOp::PlanarFill { curves } => planar_fill(model, base, curves)?,
        SurfaceOp::Fill {
            boundary,
            continuity,
        } => fill(model, base, boundary, *continuity)?,
        SurfaceOp::Ruled { first, second } => {
            let a = one_curve(model, base, first, "first")?;
            let b = one_curve(model, base, second, "second")?;
            make_ruled(model, &a, &b, tol())
                .map(|built| built.shape)
                .map_err(|e| format!("the ruled surface failed: {e}"))?
        }
        SurfaceOp::Loft { sections, closed } => loft(model, base, sections, *closed)?,
        SurfaceOp::GuidedLoft { sections, guides } => guided_loft(model, base, sections, guides)?,
        SurfaceOp::SweepTwoRails {
            profile,
            first_rail,
            second_rail,
        } => sweep_two_rails(model, base, profile, first_rail, second_rail)?,
        SurfaceOp::Sweep {
            profile,
            path,
            frame,
        } => sweep(model, base, profile, path, frame)?,
        SurfaceOp::Offset { faces, distance } => offset(model, base, faces, *distance)?,
        SurfaceOp::Blend {
            first,
            second,
            continuity,
        } => blend(model, base, first, second, *continuity)?,
        _ => unreachable!("only constructive steps reach here"),
    };
    let shape = beside(model, base, sheet.clone())?;
    Ok(Made {
        shape,
        sheet: Some(sheet),
    })
}

/// `sheet` added to the body's shape: the pieces of both in one compound.
fn beside(model: &mut Model, base: Option<&Shape>, sheet: Shape) -> Result<Shape, String> {
    let mut pieces = match base {
        Some(base) => pieces_of(model, base),
        None => Vec::new(),
    };
    pieces.extend(pieces_of(model, &sheet));
    one_or_compound(model, pieces)
}

/// A compound's members, or the shape itself.
fn pieces_of(model: &Model, shape: &Shape) -> Vec<Shape> {
    if model.kind_of(shape) == Ok(ShapeType::Compound)
        && let Ok(children) = model.children_of(shape)
    {
        return children;
    }
    vec![shape.clone()]
}

fn one_or_compound(model: &mut Model, mut pieces: Vec<Shape>) -> Result<Shape, String> {
    match pieces.len() {
        0 => Err("the step made nothing".into()),
        1 => Ok(pieces.remove(0)),
        _ => model
            .add_compound(&pieces)
            .map_err(|e| format!("gathering the sheets failed: {e}")),
    }
}

fn point(p: [f64; 3]) -> Point {
    Point::new(p[0], p[1], p[2])
}

fn direction(v: [f64; 3], what: &str) -> Result<Direction, String> {
    Direction::new(Vector::new(v[0], v[1], v[2]), tol())
        .map_err(|_| format!("{what} has no direction"))
}

/// Each source as a kernel shape: a sketch chain as a wire (open or
/// closed), a picked edge as the edge of the body it names.
fn shapes_of(
    model: &mut Model,
    base: Option<&Shape>,
    curves: &[CurveSource],
) -> Result<Vec<Shape>, String> {
    if curves.is_empty() {
        return Err("the step has no curves to build from".into());
    }
    curves
        .iter()
        .map(|curve| match curve {
            CurveSource::Sketch { plane, wire } => profile::build_wire(model, plane, wire, false),
            CurveSource::Edge(probe) => {
                let base = base.ok_or("a picked edge needs a surface in the body to be on")?;
                super::dressup::picked_edges(model, base, std::slice::from_ref(probe))?
                    .into_iter()
                    .next()
                    .ok_or_else(|| "a picked edge is no longer in the body".to_string())
            }
            CurveSource::BodyEdge {
                brep,
                edge,
                transform,
            } => {
                let other = crate::chain::absorb_shape(model, brep)?;
                let found =
                    super::dressup::picked_edges(model, &other, std::slice::from_ref(edge))?
                        .into_iter()
                        .next()
                        .ok_or_else(|| {
                            "a picked edge is no longer in the body it was picked on".to_string()
                        })?;
                match transform {
                    Some(matrix) => super::pattern::moved(model, &found, matrix),
                    None => Ok(found),
                }
            }
        })
        .collect()
}

/// The edges of the sources, in their order: a wire's in its own order.
fn edges_of(
    model: &mut Model,
    base: Option<&Shape>,
    curves: &[CurveSource],
) -> Result<Vec<Shape>, String> {
    let mut edges = Vec::new();
    for shape in shapes_of(model, base, curves)? {
        match model.kind_of(&shape) {
            Ok(ShapeType::Edge) => edges.push(shape),
            _ => edges.extend(
                explore_unique(model, &shape, ShapeType::Edge)
                    .map_err(|e| format!("reading a curve's edges failed: {e}"))?,
            ),
        }
    }
    Ok(edges)
}

fn extrude(
    model: &mut Model,
    base: Option<&Shape>,
    curves: &[CurveSource],
    along: [f64; 3],
    length: f64,
    symmetric: bool,
) -> Result<Shape, String> {
    if !(length.is_finite() && length > 0.0) {
        return Err("an extrusion needs a length above zero".into());
    }
    let dir = direction(along, "the extrusion")?;
    let vector = Vector::from(dir) * length;
    let mut sheets = Vec::new();
    for curve in shapes_of(model, base, curves)? {
        let start = if symmetric {
            let fresh = copied(model, &curve)
                .map_err(|e| format!("copying a curve failed: {e}"))?
                .shape;
            transformed(model, &fresh, Transform::translation(vector * -0.5))
                .map_err(|e| format!("moving a curve failed: {e}"))?
                .shape
        } else {
            curve
        };
        let built = make_prism(model, &start, vector, tol())
            .map_err(|e| format!("extruding a curve failed: {e}"))?;
        // A curve extruded along itself (a line along its own direction)
        // sweeps no surface at all.
        let area = ogeom::algo::surface_properties(
            model,
            &built.shape,
            ogeom::mesh::Deflection::default(),
            tol(),
        )
        .map_err(|e| format!("measuring the extruded surface failed: {e}"))?
        .mass;
        if area <= tol().confusion() * length {
            return Err(
                "a curve runs along the extrusion's direction, so it sweeps out no surface; \
                 extrude it another way"
                    .into(),
            );
        }
        sheets.push(built.shape);
    }
    one_or_compound(model, sheets)
}

fn revolve(
    model: &mut Model,
    base: Option<&Shape>,
    curves: &[CurveSource],
    origin: [f64; 3],
    axis: [f64; 3],
    angle_deg: f64,
) -> Result<Shape, String> {
    if !(angle_deg.is_finite() && angle_deg > 0.0 && angle_deg <= 360.0) {
        return Err("a revolution turns by more than 0° and at most 360°".into());
    }
    let axis = Axis::new(point(origin), direction(axis, "the axis")?);
    let mut sheets = Vec::new();
    for curve in shapes_of(model, base, curves)? {
        let built = make_revolution(model, &curve, axis, angle_deg.to_radians(), tol())
            .map_err(|e| format!("revolving a curve failed: {e}"))?;
        sheets.push(built.shape);
    }
    one_or_compound(model, sheets)
}

/// Closed sketch loops become faces as a profile's do, a loop inside
/// another a hole in it. Picked edges closing a planar loop become the face
/// on its plane.
fn planar_fill(
    model: &mut Model,
    base: Option<&Shape>,
    curves: &[CurveSource],
) -> Result<Shape, String> {
    let sketched: Vec<_> = curves
        .iter()
        .filter_map(|c| match c {
            CurveSource::Sketch { plane, wire } => Some((plane, wire)),
            CurveSource::Edge(_) | CurveSource::BodyEdge { .. } => None,
        })
        .collect();
    if sketched.len() == curves.len()
        && let Some((plane, _)) = sketched.first()
    {
        let profile = kernel_api::Profile {
            plane: **plane,
            wires: sketched.iter().map(|(_, wire)| (*wire).clone()).collect(),
        };
        let built = profile::build_profile(model, &profile)?;
        return one_or_compound(model, built.faces);
    }
    let edges = edges_of(model, base, curves)?;
    let ordered = ogeom::algo::order_edges(model, &edges, tol())
        .map_err(|e| format!("the edges do not run end to end: {e}"))?;
    let wire = make_wire(model, &ordered, tol())
        .map_err(|e| format!("the edges do not close a loop: {e}"))?
        .shape;
    let plane = find_plane(model, &wire, tol())
        .map_err(|e| format!("finding the loop's plane failed: {e}"))?
        .ok_or("the loop does not lie in one plane")?;
    // The face's own bounds are its wire; the surface only needs to span it.
    let surface = PlaneSurface::over(plane, (-1.0e6, 1.0e6), (-1.0e6, 1.0e6))
        .map_err(|e| format!("the loop's plane: {e}"))?;
    let face = make_face(model, SurfaceGeometry::Plane(surface), &[wire], tol())
        .map_err(|e| format!("filling the loop failed: {e}"))?
        .shape;
    trimmed_where_bare(model, &face, tol()).map_err(|e| format!("filling the loop failed: {e}"))?;
    Ok(face)
}

/// The kernel's continuity for a side asking `continuity`.
fn kernel_continuity(continuity: Continuity) -> ogeom::geom::Continuity {
    match continuity {
        Continuity::G0 => ogeom::geom::Continuity::C0,
        Continuity::G1 => ogeom::geom::Continuity::G1,
        Continuity::G2 => ogeom::geom::Continuity::G2,
    }
}

/// The face of `base` an edge of it belongs to: the one a step meets across
/// the edge. `None` for an edge no face holds (a sketch's).
fn face_holding(model: &Model, base: Option<&Shape>, edge: &Shape) -> Option<Shape> {
    let base = base?;
    explore_unique(model, base, ShapeType::Face)
        .ok()?
        .into_iter()
        .find(|face| {
            explore_unique(model, face, ShapeType::Edge)
                .is_ok_and(|edges| edges.iter().any(|e| e.is_same(edge)))
        })
}

/// The hole the boundary curves close, filled: a side that is an edge of
/// the body meets the face it belongs to with `continuity`, a sketch's
/// curve is touched. The face is bounded by the given edges themselves, so
/// it sews to the faces around it.
fn fill(
    model: &mut Model,
    base: Option<&Shape>,
    boundary: &[CurveSource],
    continuity: Continuity,
) -> Result<Shape, String> {
    let sides: Vec<FillBoundary> = edges_of(model, base, boundary)?
        .into_iter()
        .map(|edge| {
            let support = face_holding(model, base, &edge);
            let continuity = if support.is_some() {
                kernel_continuity(continuity)
            } else {
                ogeom::geom::Continuity::C0
            };
            FillBoundary {
                edge,
                support,
                continuity,
            }
        })
        .collect();
    make_filling_n(model, &sides, &[], FILL_TOLERANCE, tol())
        .map(|filled| filled.built.shape)
        .map_err(|e| format!("filling the boundary failed: {e}"))
}

/// How near a fill meets its sides: a distance (mm) for a gap, an angle
/// (radians) for a tangent side, a curvature (1/mm) for a curvature
/// continuous one.
const FILL_TOLERANCE: f64 = 1e-3;

/// One source as a single curve: a sketch's one chain, or a picked edge.
fn one_curve(
    model: &mut Model,
    base: Option<&Shape>,
    curve: &CurveSource,
    which: &str,
) -> Result<Shape, String> {
    let mut shapes = shapes_of(model, base, std::slice::from_ref(curve))?;
    match shapes.len() {
        1 => Ok(shapes.remove(0)),
        n => Err(format!("the {which} curve must be one chain; it has {n}")),
    }
}

fn loft(
    model: &mut Model,
    base: Option<&Shape>,
    sections: &[CurveSource],
    closed: bool,
) -> Result<Shape, String> {
    let shapes = shapes_of(model, base, sections)?;
    make_loft_surface(model, &shapes, closed, &[], false, tol())
        .map(|built| built.shape)
        .map_err(|e| format!("the loft failed: {e}"))
}

/// A surface through the sections that follows the guides, one face.
fn guided_loft(
    model: &mut Model,
    base: Option<&Shape>,
    sections: &[CurveSource],
    guides: &[CurveSource],
) -> Result<Shape, String> {
    let sections = shapes_of(model, base, sections)?;
    let guides = shapes_of(model, base, guides)?;
    make_loft_surface(model, &sections, false, &guides, false, tol())
        .map(|built| built.shape)
        .map_err(|e| format!("the guided loft failed: {e}"))
}

/// The curves as one chain, end to end: a path, a rail or a profile.
fn chain(
    model: &mut Model,
    base: Option<&Shape>,
    curves: &[CurveSource],
    what: &str,
) -> Result<Shape, String> {
    let edges = edges_of(model, base, curves)?;
    if edges.len() == 1 {
        return Ok(edges[0].clone());
    }
    let ordered = ogeom::algo::order_edges(model, &edges, tol())
        .map_err(|e| format!("the {what} does not run end to end: {e}"))?;
    make_wire(model, &ordered, tol())
        .map(|built| built.shape)
        .map_err(|e| format!("the {what} is not one chain: {e}"))
}

/// A profile swept between two rails, its ends riding them.
fn sweep_two_rails(
    model: &mut Model,
    base: Option<&Shape>,
    profile: &[CurveSource],
    first_rail: &[CurveSource],
    second_rail: &[CurveSource],
) -> Result<Shape, String> {
    let profile = chain(model, base, profile, "profile")?;
    let first = chain(model, base, first_rail, "first rail")?;
    let second = chain(model, base, second_rail, "second rail")?;
    make_sweep_two_rails(model, &profile, &first, &second, tol())
        .map(|built| built.shape)
        .map_err(|e| format!("the two-rail sweep failed: {e}"))
}

/// A round of `radius` between two faces of the body that need share no
/// edge, each cut back to where the round meets it: the two faces and the
/// round are one sheet after it, sewn to what their sheets held besides.
fn fillet_between(
    model: &mut Model,
    base: &Shape,
    first: &FaceProbe,
    second: &FaceProbe,
    radius: f64,
    flip: [bool; 2],
) -> Result<Shape, String> {
    let a = picked_face(model, base, first)?;
    let b = picked_face(model, base, second)?;
    if a.is_same(&b) {
        return Err("pick two different faces to round between".into());
    }
    let side = |face: &Shape, flip: bool| if flip { face.reversed() } else { face.clone() };
    let built = ogeom::fillet::fillet_faces(
        model,
        &side(&a, flip[0]),
        &side(&b, flip[1]),
        radius,
        true,
        tol(),
    )
    .map_err(|e| format!("rounding between the faces failed: {e}"))?;
    crate::naming::record(&built.history);
    let rounded = explore_unique(model, &built.shape, ShapeType::Face)
        .map_err(|e| format!("reading the round's faces failed: {e}"))?;
    // The sheets holding the two faces give up those faces; what is left of
    // them is sewn to the rounded sheet.
    let mut pieces = Vec::new();
    let mut joined = rounded;
    for sheet in sheets_of(model, base) {
        let faces = explore_unique(model, &sheet, ShapeType::Face)
            .map_err(|e| format!("reading a sheet's faces failed: {e}"))?;
        if !faces.iter().any(|f| f.is_same(&a) || f.is_same(&b)) {
            pieces.push(sheet);
            continue;
        }
        joined.extend(
            faces
                .into_iter()
                .filter(|f| !f.is_same(&a) && !f.is_same(&b)),
        );
    }
    let sewn = sew(model, &joined, tol()).map_err(|e| format!("joining the round failed: {e}"))?;
    crate::naming::record(&sewn.history);
    pieces.extend(sewn.shells);
    for piece in pieces_of(model, base) {
        if !matches!(
            model.kind_of(&piece),
            Ok(ShapeType::Face | ShapeType::Shell | ShapeType::Compound)
        ) {
            pieces.push(piece);
        }
    }
    one_or_compound(model, pieces)
}

/// Each profile curve swept along the path, turning about it as `frame`
/// says. An open path runs from the end nearer the profile, wherever the
/// path was drawn from.
fn sweep(
    model: &mut Model,
    base: Option<&Shape>,
    profile: &[CurveSource],
    path: &[CurveSource],
    frame: &PipeFrame,
) -> Result<Shape, String> {
    let profiles = shapes_of(model, base, profile)?;
    let path_edges = edges_of(model, base, path)?;
    let ordered = ogeom::algo::order_edges(model, &path_edges, tol())
        .map_err(|e| format!("the path does not run end to end: {e}"))?;
    let ordered = from_end_nearest(model, ordered, &profiles)?;
    let spine = make_wire(model, &ordered, tol())
        .map_err(|e| format!("the path is not one chain: {e}"))?
        .shape;
    let law = match frame {
        PipeFrame::Frenet => PipeLaw::Frenet,
        PipeFrame::Fixed => PipeLaw::Fixed,
        PipeFrame::Binormal { direction } => PipeLaw::Binormal(
            Direction::new(Vector::new(direction[0], direction[1], direction[2]), tol())
                .map_err(|_| "the sweep's binormal has no direction".to_string())?,
        ),
        PipeFrame::RotationMinimizing | PipeFrame::Auxiliary { .. } => PipeLaw::RotationMinimizing,
    };
    let mut sheets = Vec::new();
    for curve in profiles {
        let built = make_sweep_surface(model, &curve, &spine, &law, tol())
            .map_err(|e| format!("sweeping the profile failed: {e}"))?;
        sheets.push(built.shape);
    }
    one_or_compound(model, sheets)
}

/// A chain of edges walked end to end, turned round when its last end is
/// nearer the profiles than its first: the sweep starts where the profile
/// sits. A closed chain is kept as it is.
fn from_end_nearest(
    model: &Model,
    edges: Vec<Shape>,
    profiles: &[Shape],
) -> Result<Vec<Shape>, String> {
    let ends = |edge: &Shape| {
        ogeom::algo::edge_vertices(model, edge)
            .map_err(|e| format!("reading the path's ends failed: {e}"))
    };
    let (Some(first), Some(last)) = (edges.first(), edges.last()) else {
        return Ok(edges);
    };
    let (Some((start, _)), Some((_, end))) = (ends(first)?, ends(last)?) else {
        return Ok(edges);
    };
    if start.is_same(&end) {
        return Ok(edges);
    }
    let nearest = |vertex: &Shape| {
        profiles
            .iter()
            .filter_map(|p| {
                ogeom::algo::distance_between_shapes(
                    model,
                    vertex,
                    p,
                    ogeom::intersect::ExtremaOptions::default(),
                    tol(),
                )
                .ok()
                .map(|d| d.distance)
            })
            .fold(f64::INFINITY, f64::min)
    };
    if nearest(&end) < nearest(&start) {
        Ok(edges.into_iter().rev().map(|e| e.reversed()).collect())
    } else {
        Ok(edges)
    }
}

/// The face of `base` a pick names: by name, else the nearest.
pub(crate) fn picked_face(
    model: &mut Model,
    base: &Shape,
    pick: &FaceProbe,
) -> Result<Shape, String> {
    let point = point(pick.point);
    let by_name = (pick.name != 0)
        .then(|| crate::naming::find_face(model, base, pick.name, point))
        .flatten();
    match by_name {
        Some(face) => Ok(face),
        None => super::dressup::nearest_of(model, base, ShapeType::Face, point)
            .map_err(|e| format!("a picked face is no longer in the body: {e}")),
    }
}

/// The picked faces (or every sheet of the body, with none picked) copied
/// out along their normals by `distance`.
fn offset(
    model: &mut Model,
    base: Option<&Shape>,
    faces: &[FaceProbe],
    distance: f64,
) -> Result<Shape, String> {
    let base = base.ok_or("there is no surface in the body to offset yet")?;
    let mut picked = Vec::with_capacity(faces.len());
    for pick in faces {
        picked.push(picked_face(model, base, pick)?);
    }
    let sheet = match picked.len() {
        0 => return Err("pick the faces to offset".into()),
        1 => picked.remove(0),
        _ => {
            make_shell(model, &picked)
                .map_err(|e| format!("the picked faces do not make one sheet: {e}"))?
                .shape
        }
    };
    offset_sheet(model, &sheet, distance, tol())
        .map(|built| built.shape)
        .map_err(|e| format!("offsetting failed: {e}"))
}

/// A surface bridging two picked edges, meeting each edge's face with
/// `continuity`.
fn blend(
    model: &mut Model,
    base: Option<&Shape>,
    first: &EdgeProbe,
    second: &EdgeProbe,
    continuity: Continuity,
) -> Result<Shape, String> {
    let base = base.ok_or("there are no surfaces in the body to blend yet")?;
    let side = |model: &mut Model, probe: &EdgeProbe| -> Result<(Shape, Shape), String> {
        let edge = super::dressup::picked_edges(model, base, std::slice::from_ref(probe))?
            .into_iter()
            .next()
            .ok_or_else(|| "a picked edge is no longer in the body".to_string())?;
        let face = face_holding(model, Some(base), &edge)
            .ok_or_else(|| "a picked edge belongs to no face".to_string())?;
        Ok((edge, face))
    };
    let (a, fa) = side(model, first)?;
    let (b, fb) = side(model, second)?;
    let continuity = kernel_continuity(continuity);
    ogeom::fillet::make_blend_surface(model, (&a, &fa), (&b, &fb), (continuity, continuity), tol())
        .map(|built| built.shape)
        .map_err(|e| format!("blending the edges failed: {e}"))
}

/// Each sheet of the body given `thickness`: a solid apiece.
fn thicken(
    model: &mut Model,
    base: &Shape,
    thickness: f64,
    both_sides: bool,
) -> Result<Shape, String> {
    let mut solids = Vec::new();
    for piece in sheets_of(model, base) {
        let built = make_thick_sheet(model, &piece, thickness, both_sides, tol())
            .map_err(|e| format!("thickening failed: {e}"))?;
        crate::naming::record(&built.history);
        solids.push(built.shape);
    }
    if solids.is_empty() {
        return Err("the body has no sheet to thicken".into());
    }
    one_or_compound(model, solids)
}

/// The body's sheets one by one: each shell, and each face no shell holds.
fn sheets_of(model: &Model, base: &Shape) -> Vec<Shape> {
    let mut out = Vec::new();
    for piece in pieces_of(model, base) {
        match model.kind_of(&piece) {
            Ok(ShapeType::Face | ShapeType::Shell) => out.push(piece),
            Ok(ShapeType::Compound) => out.extend(sheets_of(model, &piece)),
            _ => {}
        }
    }
    out
}

/// What of the body lies on the side of the plane its normal points to.
fn trim(
    model: &mut Model,
    base: &Shape,
    origin: [f64; 3],
    normal: [f64; 3],
) -> Result<Shape, String> {
    let n = direction(normal, "the trimming plane")?;
    let at = point(origin);
    let keep = at + Vector::from(n);
    let kept = super::sweep::trim_with_halfspace(model, base, at, n, keep).map_err(|e| {
        e.replace(
            "trimming the sweep at the target plane",
            "trimming the body",
        )
    })?;
    let left = explore_unique(model, &kept, ShapeType::Face).map_or(0, |f| f.len());
    if left == 0 {
        return Err(
            "the plane leaves nothing of the body on the side kept; flip it or move it".into(),
        );
    }
    Ok(kept)
}

/// Picked faces split along curves projected onto them along their normals.
fn split(
    model: &mut Model,
    base: &Shape,
    faces: &[FaceProbe],
    curves: &[CurveSource],
) -> Result<Shape, String> {
    if faces.is_empty() {
        return Err("pick the faces to split".into());
    }
    let cuts = edges_of(model, Some(base), curves)?;
    // Sketches drawn on one plane land on the face as seen square to it,
    // as a drawing projected onto it; anything else goes to the nearest
    // point of the face.
    let normals: Vec<[f64; 3]> = curves
        .iter()
        .filter_map(|c| match c {
            CurveSource::Sketch { plane, .. } => Some(plane.normal),
            CurveSource::Edge(_) | CurveSource::BodyEdge { .. } => None,
        })
        .collect();
    let one_plane = normals.len() == curves.len()
        && normals.windows(2).all(|w| {
            let cross = [
                w[0][1] * w[1][2] - w[0][2] * w[1][1],
                w[0][2] * w[1][0] - w[0][0] * w[1][2],
                w[0][0] * w[1][1] - w[0][1] * w[1][0],
            ];
            cross.iter().all(|c| c.abs() < 1e-9)
        });
    let projection = match normals.first() {
        Some(normal) if one_plane => {
            ogeom::heal::Projection::Along(direction(*normal, "the sketch's plane")?)
        }
        _ => ogeom::heal::Projection::AlongNormals,
    };
    let mut shape = base.clone();
    for pick in faces {
        let face = picked_face(model, &shape, pick)?;
        let built = ogeom::heal::split_face(model, &shape, &face, &cuts, projection, tol())
            .map_err(|e| format!("splitting a face failed: {e}"))?;
        crate::naming::record(&built.history);
        shape = built.shape;
    }
    Ok(shape)
}

/// Faces carried past picked edges by `length`: on their own surface for
/// G2 (the natural extension), straight on tangent for G1, straight on for
/// G0.
fn extend(
    model: &mut Model,
    base: &Shape,
    edges: &[EdgeProbe],
    length: f64,
    continuity: Continuity,
) -> Result<Shape, String> {
    if edges.is_empty() {
        return Err("pick the edges to extend past".into());
    }
    let mode = match continuity {
        Continuity::G2 => Extension::Natural,
        other => Extension::Linear {
            continuity: kernel_continuity(other),
        },
    };
    // The body's sheets, each as its faces: a picked edge's face is
    // extended on its own (the kernel extends a face) and its sheet sewn
    // back together once every edge is done.
    let mut sheets: Vec<(Shape, Vec<Shape>, bool)> = Vec::new();
    for sheet in sheets_of(model, base) {
        let faces = explore_unique(model, &sheet, ShapeType::Face)
            .map_err(|e| format!("reading a sheet's faces failed: {e}"))?;
        sheets.push((sheet, faces, false));
    }
    for probe in edges {
        let current: Vec<Shape> = sheets.iter().flat_map(|(_, f, _)| f.clone()).collect();
        let all = model
            .add_compound(&current)
            .map_err(|e| format!("gathering the faces failed: {e}"))?;
        let edge = super::dressup::picked_edges(model, &all, std::slice::from_ref(probe))?
            .into_iter()
            .next()
            .ok_or_else(|| "a picked edge is no longer in the body".to_string())?;
        let holding = |model: &Model, face: &Shape| {
            explore_unique(model, face, ShapeType::Edge)
                .is_ok_and(|edges| edges.iter().any(|e| e.is_same(&edge)))
        };
        let (sheet, at) = sheets
            .iter()
            .enumerate()
            .find_map(|(s, (_, faces, _))| {
                faces
                    .iter()
                    .position(|f| holding(model, f))
                    .map(|at| (s, at))
            })
            .ok_or("the picked edge is on a solid; Extend works on surfaces")?;
        let face = sheets[sheet].1[at].clone();
        let built = extend_face(model, &face, &edge, length, mode, tol())
            .map_err(|e| format!("extending a face failed: {e}"))?;
        crate::naming::record(&built.history);
        sheets[sheet].1[at] = built.shape;
        sheets[sheet].2 = true;
    }
    let mut pieces = Vec::new();
    for (sheet, faces, changed) in sheets {
        if !changed {
            pieces.push(sheet);
        } else if faces.len() == 1 {
            pieces.extend(faces);
        } else {
            let sewn = sew(model, &faces, tol())
                .map_err(|e| format!("joining the extended faces failed: {e}"))?;
            crate::naming::record(&sewn.history);
            pieces.extend(sewn.shells);
        }
    }
    for piece in pieces_of(model, base) {
        if !matches!(
            model.kind_of(&piece),
            Ok(ShapeType::Face | ShapeType::Shell | ShapeType::Compound)
        ) {
            pieces.push(piece);
        }
    }
    one_or_compound(model, pieces)
}

/// A round of `radius` along picked edges where two faces of a sheet meet.
fn fillet(
    model: &mut Model,
    base: &Shape,
    edges: &[EdgeProbe],
    radius: f64,
) -> Result<Shape, String> {
    let picked = super::dressup::picked_edges(model, base, edges)?;
    if picked.is_empty() {
        return Err("pick the edges to round".into());
    }
    // Each sheet holding a picked edge is rounded on its own.
    let mut pieces = Vec::new();
    for sheet in sheets_of(model, base) {
        let own: Vec<Shape> = explore_unique(model, &sheet, ShapeType::Edge)
            .map_err(|e| format!("reading a sheet's edges failed: {e}"))?
            .into_iter()
            .filter(|e| picked.iter().any(|p| p.is_same(e)))
            .collect();
        if own.is_empty() {
            pieces.push(sheet);
            continue;
        }
        let built =
            ogeom::fillet::fillet_sheet_edges(model, &sheet, &own, radius, tol()).map_err(|e| {
                let e = e.to_string();
                // A round needs a face of the sheet on each side; an edge
                // with one is where separate surfaces meet, or a free edge.
                if e.contains("one face of the sheet only") || e.contains("got a Face") {
                    "the edge has a face on one side only; where two surfaces meet there, Sew \
                     them first so they share it"
                        .to_string()
                } else {
                    format!("rounding the edges failed: {e}")
                }
            })?;
        crate::naming::record(&built.history);
        pieces.push(built.shape);
    }
    for piece in pieces_of(model, base) {
        if !matches!(
            model.kind_of(&piece),
            Ok(ShapeType::Face | ShapeType::Shell | ShapeType::Compound)
        ) {
            pieces.push(piece);
        }
    }
    one_or_compound(model, pieces)
}

/// Every face of the body joined where their edges meet, or come within
/// `gap`; a shell that closes is made a solid.
fn sew_all(model: &mut Model, base: &Shape, gap: f64) -> Result<Shape, String> {
    let faces = explore_unique(model, base, ShapeType::Face)
        .map_err(|e| format!("reading the body's faces failed: {e}"))?;
    let sewn = if gap > 0.0 {
        sew_within(model, &faces, gap, tol())
    } else {
        sew(model, &faces, tol())
    }
    .map_err(|e| format!("sewing failed: {e}"))?;
    crate::naming::record(&sewn.history);
    let mut pieces = Vec::with_capacity(sewn.shells.len());
    for shell in sewn.shells {
        let closed = ogeom::algo::is_shell_closed(model, &shell).unwrap_or(false);
        pieces.push(if closed {
            make_solid(model, std::slice::from_ref(&shell))
                .map(|b| b.shape)
                .map_err(|e| format!("making the closed shell a solid failed: {e}"))?
        } else {
            shell
        });
    }
    one_or_compound(model, pieces)
}

/// The body's shape with its reflection in the plane beside it.
fn mirror(
    model: &mut Model,
    base: &Shape,
    origin: [f64; 3],
    normal: [f64; 3],
) -> Result<Shape, String> {
    let n = direction(normal, "the mirror plane")?;
    let n = Vector::from(n);
    let o = Vector::new(origin[0], origin[1], origin[2]);
    // x' = x - 2 n (n·(x - o))
    let d = 2.0 * n.dot(o);
    let mut m = [[0.0; 4]; 4];
    let nn = [n.x, n.y, n.z];
    for (i, row) in m.iter_mut().enumerate().take(3) {
        for (j, cell) in row.iter_mut().enumerate().take(3) {
            *cell = f64::from(u8::from(i == j)) - 2.0 * nn[i] * nn[j];
        }
        row[3] = d * nn[i];
    }
    m[3][3] = 1.0;
    let image = super::pattern::mirrored(model, base, &m)?;
    let mut pieces = pieces_of(model, base);
    pieces.extend(pieces_of(model, &image));
    one_or_compound(model, pieces)
}
