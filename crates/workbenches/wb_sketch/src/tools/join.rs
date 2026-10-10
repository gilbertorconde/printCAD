//! Join: a chain of curves merged into one B-spline that follows them
//! within a tolerance, starting and ending where the chain does.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use super::ToolEffect;
use crate::sketch::{BSpline, GeometryElement, Point, Sketch, Vec2D};
use crate::snap::arc_angles;

/// How close the joined spline keeps to the curves it replaces, mm, when
/// nothing else is asked for.
pub const JOIN_TOLERANCE: f32 = 0.01;

/// The most control points a joined spline takes.
const MAX_CONTROL_POINTS: usize = 120;

/// Samples along each curve of the chain.
const SAMPLES_PER_CURVE: usize = 48;

/// A curve that can be part of a chain: its end points, start to end, and
/// points along it in that direction.
fn chain_curve(sketch: &Sketch, geom: &GeometryElement) -> Option<(Uuid, Uuid, Vec<[f64; 2]>)> {
    let at = |id: Uuid| sketch.point_position(id);
    let f = |p: Vec2D| [f64::from(p.x), f64::from(p.y)];
    let n = SAMPLES_PER_CURVE;
    match geom {
        GeometryElement::Line(l) => {
            let (a, b) = (f(at(l.start)?), f(at(l.end)?));
            let points = (0..=n)
                .map(|i| {
                    let t = i as f64 / n as f64;
                    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
                })
                .collect();
            Some((l.start, l.end, points))
        }
        GeometryElement::Arc(arc) => {
            let (c, s, e) = (at(arc.center)?, at(arc.start)?, at(arc.end)?);
            let (start_angle, sweep) = arc_angles((s - c).to_glam(), (e - c).to_glam());
            let r = f64::from((s - c).to_glam().length());
            let (cx, cy) = (f64::from(c.x), f64::from(c.y));
            let mut points: Vec<[f64; 2]> = (0..=n)
                .map(|i| {
                    let a = f64::from(start_angle) + f64::from(sweep) * i as f64 / n as f64;
                    [cx + r * a.cos(), cy + r * a.sin()]
                })
                .collect();
            // The ends exactly where the arc's points are.
            points[0] = f(s);
            points[n] = f(e);
            Some((arc.start, arc.end, points))
        }
        GeometryElement::Ellipse(e) => {
            let ends = e.arc?;
            let mut points: Vec<[f64; 2]> = e.points(sketch, n)?.into_iter().map(f).collect();
            points[0] = f(at(ends.start)?);
            points[n] = f(at(ends.end)?);
            Some((ends.start, ends.end, points))
        }
        GeometryElement::Conic(c) => {
            let mut points: Vec<[f64; 2]> = c.points(sketch, n)?.into_iter().map(f).collect();
            points[0] = f(at(c.start)?);
            points[n] = f(at(c.end)?);
            Some((c.start, c.end, points))
        }
        GeometryElement::BSpline(b) if !b.periodic => {
            let (first, last) = (*b.control_points.first()?, *b.control_points.last()?);
            let points = b.points(sketch, n)?.into_iter().map(f).collect();
            Some((first, last, points))
        }
        _ => None,
    }
}

/// A curve of the selection: its id, its end points and points along it.
type ChainCurve = (Uuid, Uuid, Uuid, Vec<[f64; 2]>);

/// The selected curves walked end to end.
struct Walk {
    /// Each curve with its points turned to run along the chain.
    curves: Vec<(Uuid, Vec<[f64; 2]>)>,
    /// Where the chain starts and ends: one point for a closed one.
    start: Uuid,
    end: Uuid,
}

/// The selected curves in chain order. Deterministic: an open chain starts
/// at the free end of whichever of its end curves comes first in the
/// sketch. `None` when the curves branch or fall apart.
fn walk(curves: &[ChainCurve]) -> Option<Walk> {
    let mut touching: HashMap<Uuid, Vec<usize>> = HashMap::new();
    for (i, (_, a, b, _)) in curves.iter().enumerate() {
        touching.entry(*a).or_default().push(i);
        touching.entry(*b).or_default().push(i);
    }
    if touching.values().any(|v| v.len() > 2) {
        return None;
    }
    let free = |p: &Uuid| touching.get(p).is_some_and(|v| v.len() == 1);
    let start = curves
        .iter()
        .find_map(|(_, a, b, _)| {
            if free(a) {
                Some(*a)
            } else if free(b) {
                Some(*b)
            } else {
                None
            }
        })
        .unwrap_or(curves[0].1);
    let mut used = vec![false; curves.len()];
    let mut out = Vec::with_capacity(curves.len());
    let mut current = start;
    while out.len() < curves.len() {
        let i = touching
            .get(&current)?
            .iter()
            .copied()
            .find(|i| !used[*i])?;
        used[i] = true;
        let (id, a, b, points) = &curves[i];
        let (exit, points) = if *a == current {
            (*b, points.clone())
        } else {
            (*a, points.iter().rev().copied().collect())
        };
        out.push((*id, points));
        current = exit;
    }
    Some(Walk {
        curves: out,
        start,
        end: current,
    })
}

/// Merge the selected curves, a chain end to end, into one cubic B-spline
/// that keeps within `tolerance` of them. The spline starts and ends on
/// the chain's own end points, so what met the chain there meets the
/// spline; the curves and the points only they used go.
pub fn join(sketch: &mut Sketch, selected: &HashSet<Uuid>, tolerance: f32) -> ToolEffect {
    let external = sketch.external_ids();
    let curves: Vec<ChainCurve> = sketch
        .geometry
        .iter()
        .filter(|g| selected.contains(&g.id()) && !matches!(g, GeometryElement::Point(_)))
        .filter(|g| !external.contains(&g.id()))
        .filter_map(|g| {
            let (a, b, points) = chain_curve(sketch, g)?;
            Some((g.id(), a, b, points))
        })
        .collect();
    let picked = sketch
        .geometry
        .iter()
        .filter(|g| selected.contains(&g.id()) && !matches!(g, GeometryElement::Point(_)))
        .count();
    if curves.len() < 2 {
        return ToolEffect::log("Select two or more curves that meet end to end to join");
    }
    if curves.len() != picked {
        return ToolEffect::log(
            "Only lines, arcs, arcs of conics and open splines join; leave the rest out",
        );
    }
    let Some(Walk {
        curves: chain,
        start,
        end,
    }) = walk(&curves)
    else {
        return ToolEffect::log("The curves must meet end to end in one chain, without branches");
    };
    let mut samples: Vec<[f64; 2]> = Vec::new();
    for (_, points) in &chain {
        let skip = usize::from(!samples.is_empty());
        samples.extend(points.iter().skip(skip).copied());
    }
    // The fit's knots are the even ones a spline takes when it has none of
    // its own.
    let Some((_, control)) =
        sketch_solver::spline::fit_chain(&samples, f64::from(tolerance), MAX_CONTROL_POINTS)
    else {
        return ToolEffect::log(format!(
            "No spline follows these curves within {tolerance} mm; a sharp corner needs a looser tolerance"
        ));
    };
    let last = control.len() - 1;
    let ids: Vec<Uuid> = control
        .iter()
        .enumerate()
        .map(|(i, p)| match i {
            0 => start,
            i if i == last => end,
            _ => sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(
                p[0] as f32,
                p[1] as f32,
            )))),
        })
        .collect();
    let construction = chain.iter().all(|(id, _)| sketch.is_construction(*id));
    let n = ids.len();
    let joined = sketch.add_geometry(GeometryElement::BSpline(BSpline::new(ids, false)));
    sketch.set_construction(joined, construction);
    let gone: Vec<Uuid> = chain.iter().map(|(id, _)| *id).collect();
    sketch.remove_geometry_cascade(&gone);
    ToolEffect::changed(format!(
        "Joined {} curves into a spline of {n} control points",
        gone.len()
    ))
}
