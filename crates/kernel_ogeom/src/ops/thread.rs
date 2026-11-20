//! A thread cut into a cylindrical face of the solid: the tooth space
//! swept along a helix about the face's axis, kept to the face's length
//! (and its run-out), with a chamfer at its starting end when asked.

use kernel_api::{
    FaceProbe, FaceThread, Profile, ProfilePlane, ProfileSegment, ProfileWire, RevolveTermination,
    SweepKind,
};
use ogeom::geom::SurfaceGeometry;
use ogeom::math::{Point, Vector};
use ogeom::mesh::{Deflection, triangulate_face};
use ogeom::topo::{Model, NodeData, Shape};

use super::{common_pieces, tol, wrap_pieces};
use crate::convert::{array, point};

/// Where a thread goes: the face's axis, radius and length, which way it
/// faces and the end it starts from.
struct Seat {
    /// The axis where the thread starts, on the face's starting end.
    start: Point,
    /// Along the axis, into the face from its starting end.
    into: Vector,
    /// Square to the axis, out toward where the face was picked.
    across: Vector,
    radius: f64,
    /// The face's length along its axis.
    length: f64,
    /// A shaft's face (outward normal away from the axis), not a bore's.
    external: bool,
}

/// The tool a [`kernel_api::SolidOp::Thread`] cuts out of `base`: the
/// groove, kept within the face's length and run out at its end, and the
/// chamfer at its start when asked; several solids in one compound.
pub fn tool(
    model: &mut Model,
    base: &Shape,
    face: &FaceProbe,
    form: &FaceThread,
) -> Result<Shape, String> {
    let FaceThread {
        pitch,
        depth,
        flank_deg,
        length,
        left_handed,
        chamfer,
        run_out,
        ..
    } = *form;
    if !(pitch > 0.0 && pitch.is_finite()) {
        return Err("the thread's pitch must be more than zero".into());
    }
    if !(depth > 0.0 && depth.is_finite()) {
        return Err("the thread's depth must be more than zero".into());
    }
    if !(flank_deg > 0.0 && flank_deg < 180.0) {
        return Err("the angle between the thread's flanks must lie between 0° and 180°".into());
    }
    if length.is_some_and(|l| l.is_nan() || l <= 0.0) {
        return Err("the thread's length must be more than zero".into());
    }
    if run_out.is_nan() || run_out < 0.0 {
        return Err("the thread's run-out must not be negative".into());
    }
    let seat = seat(model, base, face, form.from_far_end)?;
    if !seat.external && depth >= seat.radius * 4.0 {
        return Err("the thread is deeper than the bore could hold".into());
    }
    if seat.external && depth >= seat.radius {
        return Err(format!(
            "the thread, {depth:.2} mm deep, reaches the axis of a shaft {:.2} mm across",
            seat.radius * 2.0
        ));
    }
    let run = length.map_or(seat.length, |l| l.min(seat.length));
    let run_out = run_out.min(run);

    let r = seat.radius;
    // The groove from `open` (just clear of the face, outside the
    // material) to `deep` (its root, in it).
    let margin = 0.1 * pitch;
    let (open, deep) = if seat.external {
        (r + margin, r - depth)
    } else {
        ((r - margin).max(0.5 * r), r + depth)
    };
    let section = ProfilePlane {
        origin: array(seat.start),
        x_axis: array(seat.across),
        y_axis: array(seat.into),
        normal: array(seat.across.cross(seat.into)),
    };
    let crest = pitch / 16.0;
    let half_flank = (flank_deg * 0.5).to_radians().tan();
    let flank = (crest + (deep - open).abs() * half_flank).min(0.45 * pitch);
    let at = -pitch;
    let groove = polygon(&[
        [open, at - flank],
        [deep, at - crest],
        [deep, at + crest],
        [open, at + flank],
    ]);
    let sweep = super::sweep::build_tool(
        model,
        None,
        &Profile {
            plane: section,
            wires: vec![groove],
        },
        &SweepKind::Helix {
            axis_origin: [0.0, 0.0],
            axis_dir: [0.0, 1.0],
            pitch,
            height: run + 2.0 * pitch,
            left_handed,
            cone_angle_deg: 0.0,
            reversed: false,
            turns: None,
            growth: None,
            through_all: false,
        },
    )?;

    // What of the sweep stays: the face's length from its start to where
    // the thread ends, less a cone running the groove out over the
    // run-out.
    let wide = open.max(deep) + pitch;
    let fade = run - run_out;
    let kept = if run_out > 0.0 {
        if seat.external {
            vec![
                [0.0, 0.0],
                [wide, 0.0],
                [wide, run],
                [r, run],
                [deep, fade],
                [0.0, fade],
            ]
        } else {
            vec![
                [0.0, 0.0],
                [wide, 0.0],
                [wide, fade],
                [deep, fade],
                [r, run],
                [0.0, run],
            ]
        }
    } else {
        vec![[0.0, 0.0], [wide, 0.0], [wide, run], [0.0, run]]
    };
    let clip = turned(model, section, &kept)?;
    let mut pieces = common_pieces(model, &sweep, &clip)?;
    if pieces.is_empty() {
        return Err("the thread's groove misses the face".into());
    }
    if chamfer {
        // The 45° cone from the groove's root at the starting end out
        // just past the face, begun a pitch before it.
        let before = -pitch;
        let corner = if seat.external {
            let out = r + margin;
            vec![[deep + before, before], [out, before], [out, out - deep]]
        } else {
            let inner = (r - margin - pitch).max(0.0);
            vec![
                [inner, before],
                [deep - before, before],
                [inner, deep - inner],
            ]
        };
        pieces.push(turned(model, section, &corner)?);
    }
    wrap_pieces(model, pieces)
}

/// The cylindrical face `probe` finds on `base`, and where on it a thread
/// starts.
fn seat(
    model: &mut Model,
    base: &Shape,
    probe: &FaceProbe,
    from_far_end: bool,
) -> Result<Seat, String> {
    let face = crate::probe::face_for(model, base, probe, crate::probe::Reach::Pick)?;
    let (on_face, outward) = crate::probe::outward_at(model, &face, point(probe.point))?;
    let Some(NodeData::Face(data)) = model.node(&face).map(|n| n.data()) else {
        return Err("the face holds no face data".into());
    };
    let placement = face.transform(model.datums()).map_err(|e| e.to_string())?;
    let (origin, axis, radius) = match model.geometry().surface(data.surface) {
        Some(SurfaceGeometry::Cylinder(cylinder)) => {
            let c = cylinder.cylinder();
            (
                placement.apply(c.frame().origin()),
                placement.apply_vector(c.frame().z().vector()),
                c.radius(),
            )
        }
        _ => {
            return Err(
                "a thread goes on a cylindrical face: pick a shaft's or a bore's round face".into(),
            );
        }
    };
    let axis = axis / axis.magnitude().max(1e-300);
    let along = |p: Point| (p - origin).dot(axis);
    let mesh = triangulate_face(model, &face, Deflection::default(), tol())
        .map_err(|e| format!("measuring the face's length failed: {e}"))?;
    let (lo, hi) = mesh
        .positions
        .iter()
        .map(|p| along(*p))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), t| {
            (lo.min(t), hi.max(t))
        });
    if hi <= lo {
        return Err("the face has no length along its axis".into());
    }
    let picked = along(on_face);
    let near_low = (picked - lo).abs() <= (hi - picked).abs();
    let (start, into) = if near_low != from_far_end {
        (lo, axis)
    } else {
        (hi, -axis)
    };
    let radial = (on_face - origin) - axis * picked;
    let across = if radial.magnitude() > 1e-9 {
        radial / radial.magnitude()
    } else {
        any_square_to(axis)
    };
    Ok(Seat {
        start: origin + axis * start,
        into,
        across,
        radius,
        length: hi - lo,
        external: outward.dot(radial) > 0.0,
    })
}

/// A unit vector square to `v`.
fn any_square_to(v: Vector) -> Vector {
    let other = if v.x.abs() < 0.9 {
        Vector::new(1.0, 0.0, 0.0)
    } else {
        Vector::new(0.0, 1.0, 0.0)
    };
    let s = v.cross(other);
    s / s.magnitude().max(1e-300)
}

/// A closed polygon through `corners`, in order, a corner repeating the
/// one before it left out.
fn polygon(corners: &[[f64; 2]]) -> ProfileWire {
    let mut kept: Vec<[f64; 2]> = Vec::with_capacity(corners.len());
    for c in corners {
        let same = |k: &[f64; 2]| (k[0] - c[0]).abs() < 1e-9 && (k[1] - c[1]).abs() < 1e-9;
        if !kept.last().is_some_and(same) {
            kept.push(*c);
        }
    }
    if kept.len() > 1
        && (kept[0][0] - kept[kept.len() - 1][0]).abs() < 1e-9
        && (kept[0][1] - kept[kept.len() - 1][1]).abs() < 1e-9
    {
        kept.pop();
    }
    ProfileWire::new(
        (0..kept.len())
            .map(|i| ProfileSegment::Line {
                start: kept[i],
                end: kept[(i + 1) % kept.len()],
            })
            .collect(),
    )
}

/// The solid `corners` (x out from the axis, y along it, in `section`)
/// make turned a whole turn about the section's Y axis.
fn turned(model: &mut Model, section: ProfilePlane, corners: &[[f64; 2]]) -> Result<Shape, String> {
    super::sweep::build_tool(
        model,
        None,
        &Profile {
            plane: section,
            wires: vec![polygon(corners)],
        },
        &SweepKind::Revolve {
            axis_origin: [0.0, 0.0],
            axis_dir: [0.0, 1.0],
            angle_deg: 360.0,
            second_angle_deg: None,
            midplane: false,
            reversed: false,
            termination: RevolveTermination::Angle,
        },
    )
}
