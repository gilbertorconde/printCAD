//! A sketch carried onto faces of the solid along its plane's normal.
//!
//! Each picked face is split along the curves where they land on it; the
//! pieces the sketch's regions cover are what an emboss raises and a
//! deboss sinks: each piece thickened into a slab of its own, out of the
//! solid or into it, then fused on or cut away. A piece is told to be in a
//! region by a point inside it, where it faces the sketch's plane, carried
//! back onto that plane.

use kernel_api::{FaceProbe, Profile, ProjectMode};
use ogeom::algo::{Containment, classify_in_solid, classify_on_face, copied};
use ogeom::heal::{Projection, split_face};
use ogeom::math::{Direction, Point, Vector};
use ogeom::mesh::{Deflection, triangulate_face};
use ogeom::offset::make_thick_sheet;
use ogeom::topo::{Model, Shape};

use super::tol;
use crate::profile::{self, BuiltWire};

pub fn apply(
    model: &mut Model,
    base: &Shape,
    sketch: &Profile,
    faces: &[FaceProbe],
    mode: ProjectMode,
    height: f64,
) -> Result<Shape, String> {
    if faces.is_empty() {
        return Err("pick the faces to project the sketch onto".into());
    }
    if mode != ProjectMode::Split && !(height.is_finite() && height > 0.0) {
        return Err("the height must be above zero".into());
    }
    let built = profile::build_profile(model, sketch)?;
    let along = profile::plane_normal(&sketch.plane)
        .map_err(|e| format!("the sketch's plane has no normal: {e}"))?;
    let wires: Vec<&BuiltWire> = built
        .groups
        .iter()
        .flat_map(|g| std::iter::once(&g.outer).chain(&g.holes))
        .collect();

    // The picked faces split one after another; `pieces` holds what became
    // of them so far, carried through every later split.
    let mut shape = base.clone();
    let mut pieces: Vec<Shape> = Vec::new();
    let mut cut_any = false;
    for pick in faces {
        let face = super::surface::picked_face(model, &shape, pick)?;
        if pieces.iter().any(|p| p.node() == face.node()) {
            continue;
        }
        let cuts: Vec<Shape> = wires
            .iter()
            .filter(|w| reaches(model, &face, w, along))
            .flat_map(|w| w.edges.iter().cloned())
            .collect();
        if cuts.is_empty() {
            // Nothing lands on it: it is wholly in a region or wholly out.
            pieces.push(face);
            continue;
        }
        let split = split_face(model, &shape, &face, &cuts, Projection::Along(along), tol())
            .map_err(|e| format!("splitting a face along the sketch failed: {e}"))?;
        crate::naming::record(&split.history);
        let mut carried: Vec<Shape> = pieces
            .iter()
            .flat_map(|p| split.history.trace(p).to_vec())
            .collect();
        carried.extend(split.history.modified(&face).iter().cloned());
        pieces = carried;
        shape = split.shape;
        cut_any = true;
    }
    if !cut_any {
        return Err(
            "the sketch's curves do not land on the picked faces along its plane's normal".into(),
        );
    }
    if mode == ProjectMode::Split {
        return Ok(shape);
    }

    let mut covered = Vec::new();
    for piece in pieces {
        let Some(inside) = point_inside(model, &piece, sketch, along) else {
            continue;
        };
        let on_plane = onto_plane(sketch, along, inside);
        let in_region = built.faces.iter().any(|region| {
            classify_on_face(model, region, on_plane, Deflection::default(), tol())
                .is_ok_and(|c| c == Containment::In)
        });
        if in_region {
            covered.push((piece, inside));
        }
    }
    if covered.is_empty() {
        return Err("no region of the sketch lands on the picked faces".into());
    }

    let raise = mode == ProjectMode::Emboss;
    let kind = if raise {
        kernel_api::BoolKind::Fuse
    } else {
        kernel_api::BoolKind::Cut
    };
    for (piece, inside) in covered {
        let slab = slab(model, &piece, inside, height, raise)?;
        shape = super::combine_solids(model, &shape, &slab, kind)?;
    }
    Ok(shape)
}

/// Whether some point of `wire`, carried along `along` either way, meets
/// `face` as drawn.
fn reaches(model: &Model, face: &Shape, wire: &BuiltWire, along: Direction) -> bool {
    let Ok(drawn) = triangulate_face(model, face, Deflection::default(), tol()) else {
        return true;
    };
    let mut points = Vec::new();
    for edge in &wire.edges {
        if let Ok(line) = ogeom::mesh::polyline_of_edge(model, edge, Deflection::default(), tol()) {
            points.extend(line);
        }
    }
    points.iter().any(|&p| {
        drawn.triangles.iter().any(|t| {
            let [a, b, c] = t.map(|i| drawn.positions[i as usize]);
            super::sweep::ray_triangle(p, along, a, b, c).is_some()
        })
    })
}

/// A point well inside `face` where it faces the sketch's plane: the middle
/// of its largest drawn triangle turned toward the plane, or of its largest
/// one where none is.
///
/// A piece wrapping round a solid also covers the far side, whose points
/// carried onto the plane land in the same regions as the near side's: a
/// point there would take the whole wrap for a region. The far side faces
/// away from the plane, so it is passed over.
fn point_inside(model: &Model, face: &Shape, sketch: &Profile, along: Direction) -> Option<Point> {
    let drawn = triangulate_face(model, face, Deflection::default(), tol()).ok()?;
    let o = sketch.plane.origin;
    let origin = Point::new(o[0], o[1], o[2]);
    let n = along.vector();
    let triangles: Vec<(Point, f64, bool)> = drawn
        .triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| drawn.positions[i as usize]);
            let middle = Point::new(
                (a.x + b.x + c.x) / 3.0,
                (a.y + b.y + c.y) / 3.0,
                (a.z + b.z + c.z) / 3.0,
            );
            // Wound outward, so its normal faces the plane where it points
            // back across the plane's normal from the side it stands on.
            let normal = (b - a).cross(c - a);
            let facing = (middle - origin).dot(n) * normal.dot(n) <= 0.0;
            (middle, normal.magnitude(), facing)
        })
        .collect();
    let largest = |facing_only: bool| {
        triangles
            .iter()
            .filter(|t| t.2 || !facing_only)
            .max_by(|x, y| x.1.total_cmp(&y.1))
            .map(|t| t.0)
    };
    largest(true).or_else(|| largest(false))
}

/// `p` carried along `along` onto the sketch's plane.
fn onto_plane(sketch: &Profile, along: Direction, p: Point) -> Point {
    let o = sketch.plane.origin;
    let n = along.vector();
    let off = (p - Point::new(o[0], o[1], o[2])).dot(n);
    p - n * off
}

/// `piece` thickened by `height` out of the solid (`raise`) or into it.
fn slab(
    model: &mut Model,
    piece: &Shape,
    inside: Point,
    height: f64,
    raise: bool,
) -> Result<Shape, String> {
    let (at, outward) = crate::probe::outward_at(model, piece, inside)?;
    let outward = outward * (1.0 / outward.magnitude().max(1e-300));
    let toward: Vector = if raise { outward } else { -outward };
    let middle = at + toward * (height * 0.5);
    let sheet = copied(model, piece)
        .map_err(|e| format!("copying a region failed: {e}"))?
        .shape;
    let mut last = String::new();
    for thickness in [height, -height] {
        let built = match make_thick_sheet(model, &sheet, thickness, false, tol()) {
            Ok(built) => built,
            Err(e) => {
                last = format!("thickening a region failed: {e}");
                continue;
            }
        };
        let holds = classify_in_solid(model, &built.shape, middle, Deflection::default(), tol())
            .is_ok_and(|c| c == Containment::In);
        if holds {
            crate::naming::record(&built.history);
            return Ok(built.shape);
        }
    }
    Err(if last.is_empty() {
        "a region's slab came out on the wrong side of its face".into()
    } else {
        last
    })
}
