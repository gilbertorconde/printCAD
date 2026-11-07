//! Exact rational carriers of analytic curves, independent of model identities.

use std::f64::consts::FRAC_PI_2;

use crate::bspline::SplineBasis;
use crate::conic::ConicArcParams;
use crate::curves::{ellipse_arc_sweep, ellipse_axes, hyperbola_axes, parabola_frame};
use crate::world::{WorldPoint, WorldVec};

fn quadratic(poles: Vec<WorldPoint>, weights: Vec<f64>) -> (Vec<WorldPoint>, SplineBasis) {
    (
        poles,
        SplineBasis {
            knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            weights,
        },
    )
}

/// Rational quadratic pieces of at most π/2. The middle pole is the
/// intersection of endpoint tangents and its weight is cos(half sweep).
pub fn elliptic(
    center: WorldPoint,
    u: WorldVec,
    v: WorldVec,
    from: f64,
    sweep: f64,
) -> Result<(Vec<WorldPoint>, SplineBasis), String> {
    let count = (sweep.abs() / FRAC_PI_2).ceil();
    if !count.is_finite() || !(1.0..=4096.0).contains(&count) {
        return Err("Conversion needs a nonzero sweep and at most 4096 quadratic spans".into());
    }
    let n = count as usize;
    let step = sweep / n as f64;
    let at = |t: f64| u.0 * t.cos() + v.0 * t.sin();
    let weight = (step * 0.5).cos();
    let mut poles = vec![center + WorldVec::from_dvec2(at(from))];
    let mut weights = vec![1.0];
    let mut knots = vec![0.0; 3];
    for i in 0..n {
        let a = from + step * i as f64;
        poles.push(WorldPoint::from_dvec2(
            center.0 + at(a + step * 0.5) / weight,
        ));
        poles.push(WorldPoint::from_dvec2(
            center.0 + at(from + step * (i + 1) as f64),
        ));
        weights.extend([weight, 1.0]);
        if i + 1 < n {
            knots.extend([(i + 1) as f64 / n as f64; 2]);
        }
    }
    knots.extend([1.0; 3]);
    Ok((poles, SplineBasis { knots, weights }))
}

/// Exact carrier of a resolved conic span. Endpoints come from the host's
/// geometry, preserving their positions without assigning model identities.
pub fn conic(
    params: ConicArcParams,
    start: WorldPoint,
    end: WorldPoint,
) -> Result<(Vec<WorldPoint>, SplineBasis), String> {
    match params {
        ConicArcParams::Ellipse {
            center,
            focus1,
            semi_minor,
            t_start,
            t_end,
            clockwise,
        } => {
            let (u, v) =
                ellipse_axes(center, focus1, semi_minor).ok_or("The ellipse has no axes")?;
            elliptic(
                center,
                u,
                v,
                t_start.0,
                ellipse_arc_sweep(t_start, t_end, clockwise).0,
            )
        }
        ConicArcParams::Hyperbola {
            center,
            focus1,
            semi_minor,
            t_start,
            t_end,
            branch,
        } => {
            let (u, v) =
                hyperbola_axes(center, focus1, semi_minor).ok_or("The hyperbola has no axes")?;
            let (a, b) = (t_start.get(), t_end.get());
            let mid = (a + b) * 0.5;
            let w = ((b - a) * 0.5).cosh();
            let q = center.0 + (u.0 * (branch.sign() * mid.cosh()) + v.0 * mid.sinh()) / w;
            if !q.is_finite() || !w.is_finite() {
                return Err("Hyperbola conversion exceeds finite coordinates".into());
            }
            Ok(quadratic(
                vec![start, WorldPoint::from_dvec2(q), end],
                vec![1.0, w, 1.0],
            ))
        }
        ConicArcParams::Parabola {
            vertex,
            focus,
            u_start,
            u_end,
        } => {
            let frame = parabola_frame(vertex, focus).ok_or("The parabola has no axis")?;
            let (a, b) = (u_start.get(), u_end.get());
            // Tangent intersection in the local frame avoids cancellation at a
            // translated vertex.
            let q = vertex.0
                + frame.axis.0 * (a * b / (4.0 * frame.focal_length.0))
                + frame.transverse.0 * ((a + b) * 0.5);
            if !q.is_finite() {
                return Err("Parabola conversion exceeds finite coordinates".into());
            }
            Ok(quadratic(
                vec![start, WorldPoint::from_dvec2(q), end],
                vec![1.0; 3],
            ))
        }
    }
}
