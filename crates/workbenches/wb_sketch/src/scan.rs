//! Scans cut by the sketch plane: the chains where every visible scan
//! crosses it, drawn while the sketch is open as a guide, and fitted with
//! lines and arcs into the sketch's own geometry on request.

use core_document::{BodyId, Document};
use kernel_api::{ChainFit, Drawing2d, DrawingCurve, KernelQueries};

use crate::dxf::{Added, add_drawing};
use crate::sketch::{Sketch, SketchPlane};

/// Chains shorter than this, in mm, are noise and left out.
const SHORTEST_MM: f64 = 1.0;

/// One chain of a scan's section, in the sketch's coordinates.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Chain2 {
    pub points: Vec<[f64; 2]>,
    pub closed: bool,
}

impl Chain2 {
    fn length(&self) -> f64 {
        let n = self.points.len();
        let spans = if self.closed { n } else { n.saturating_sub(1) };
        (0..spans)
            .map(|i| {
                let (a, b) = (self.points[i], self.points[(i + 1) % n]);
                (b[0] - a[0]).hypot(b[1] - a[1])
            })
            .sum()
    }
}

/// The scans the sketch plane cuts: every visible scan body.
fn scans(document: &Document) -> Vec<BodyId> {
    document
        .bodies()
        .iter()
        .map(|b| b.id)
        .filter(|b| document.body_scan(*b).is_some())
        .filter(|b| document.imported_body_effective_visible(*b))
        .collect()
}

/// What the chains are cut from: the plane, and each scan's geometry and
/// where it stands. `None` when there is no scan to cut.
pub(crate) fn key(document: &Document, plane: &SketchPlane) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let bodies = scans(document);
    if bodies.is_empty() {
        return None;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{plane:?}").hash(&mut h);
    for b in bodies {
        b.0.hash(&mut h);
        document
            .imported_geometry(b)
            .map(|g| g.revision)
            .hash(&mut h);
        format!("{:?}", document.body_placement(b).rows()).hash(&mut h);
    }
    Some(h.finish())
}

/// Where every visible scan crosses `plane` (as the scene has it), in the
/// sketch's coordinates.
pub(crate) fn chains(
    document: &Document,
    kernel: &dyn KernelQueries,
    plane: &SketchPlane,
) -> Vec<Chain2> {
    let mut out = Vec::new();
    for body in scans(document) {
        let Some((mesh, _)) = document.local_geometry(body) else {
            continue;
        };
        let local = crate::placed_plane(plane, &document.body_placement(body).inverse());
        let Ok(stack) = kernel.mesh_sections(&mesh, local.origin, local.normal, &[0.0]) else {
            continue;
        };
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        for chain in stack.into_iter().flatten() {
            let on = Chain2 {
                points: chain
                    .points
                    .iter()
                    .map(|p| {
                        let d = [
                            p[0] - local.origin[0],
                            p[1] - local.origin[1],
                            p[2] - local.origin[2],
                        ];
                        [dot(d, local.x_axis), dot(d, local.y_axis)]
                    })
                    .collect(),
                closed: chain.closed,
            };
            if on.length() >= SHORTEST_MM {
                out.push(on);
            }
        }
    }
    out
}

/// How a scan's chains are fitted into the sketch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Fitting {
    pub fit: ChainFit,
    /// Made construction: guides to draw over rather than profile.
    pub construction: bool,
}

/// Every chain fitted with lines and arcs and added to `sketch`, ends
/// that meet sharing one point. The chains the kernel could not fit are
/// counted, not added.
pub(crate) fn fit_into(
    sketch: &mut Sketch,
    kernel: &dyn KernelQueries,
    chains: &[Chain2],
    fitting: Fitting,
) -> (Added, usize) {
    let mut curves = Vec::new();
    let mut failed = 0;
    for chain in chains {
        let fit = ChainFit {
            closed: chain.closed,
            ..fitting.fit
        };
        match kernel.fit_chain_2d(&chain.points, &fit) {
            Ok(shape) => curves.push(DrawingCurve {
                hidden: fitting.construction,
                shape,
            }),
            Err(_) => failed += 1,
        }
    }
    let drawing = Drawing2d {
        unit_mm: None,
        curves,
    };
    let before: std::collections::HashSet<uuid::Uuid> =
        sketch.geometry.iter().map(|g| g.id()).collect();
    let added = add_drawing(sketch, &drawing, 1.0);
    keep_points(sketch, &before, chains, fitting.fit.tolerance * 3.0);
    (added, failed)
}

/// How far a point is from a curve.
type Distance = Box<dyn Fn([f64; 2]) -> f64>;

/// At most this many measured points are kept per curve.
const KEPT: usize = 100;

/// Each chain point kept on the new curve nearest it, within `reach`, as
/// the soft data that settles what the curve's constraints leave free.
fn keep_points(
    sketch: &mut Sketch,
    before: &std::collections::HashSet<uuid::Uuid>,
    chains: &[Chain2],
    reach: f64,
) {
    let curves = curve_distances(sketch, |id| !before.contains(&id));
    let mut kept: std::collections::HashMap<uuid::Uuid, Vec<[f64; 2]>> = Default::default();
    for p in chains.iter().flat_map(|c| &c.points) {
        let nearest = curves
            .iter()
            .map(|(id, d)| (*id, d(*p)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((id, d)) = nearest
            && d <= reach
        {
            kept.entry(id).or_default().push(*p);
        }
    }
    let mut soft: Vec<crate::sketch::SoftData> = kept
        .into_iter()
        .map(|(element, points)| {
            let step = points.len().div_ceil(KEPT).max(1);
            crate::sketch::SoftData {
                element,
                points: points.into_iter().step_by(step).collect(),
            }
        })
        .collect();
    soft.sort_by_key(|s| s.element);
    sketch.soft.extend(soft);
}

/// How far a point is from the sketch's nearest drawn curve (lines,
/// circles and arcs, construction and projections left out); `None` when
/// it has none.
pub(crate) fn distance_to_sketch(sketch: &Sketch) -> impl Fn([f64; 2]) -> Option<f64> {
    let curves = curve_distances(sketch, |id| {
        !sketch.is_construction(id) && !sketch.external.contains_key(&id)
    });
    move |p| curves.iter().map(|(_, d)| d(p)).min_by(f64::total_cmp)
}

/// Each line, circle and arc `keep` takes, and how far a point is from it.
fn curve_distances(
    sketch: &Sketch,
    keep: impl Fn(uuid::Uuid) -> bool,
) -> Vec<(uuid::Uuid, Distance)> {
    use crate::sketch::GeometryElement;
    let at = |id| {
        sketch
            .point_position(id)
            .map(|p: crate::sketch::Vec2D| [p.x, p.y])
    };
    sketch
        .geometry
        .iter()
        .filter(|g| keep(g.id()))
        .filter_map(|g| -> Option<(uuid::Uuid, Distance)> {
            match g {
                GeometryElement::Line(l) => {
                    let (a, b) = (at(l.start)?, at(l.end)?);
                    Some((l.id, Box::new(move |p| to_segment(p, a, b))))
                }
                GeometryElement::Circle(c) => {
                    let (o, r) = (at(c.center)?, c.radius);
                    Some((
                        c.id,
                        Box::new(move |p| ((p[0] - o[0]).hypot(p[1] - o[1]) - r).abs()),
                    ))
                }
                GeometryElement::Arc(arc) => {
                    let (o, r) = (at(arc.center)?, arc.radius);
                    let (s, e) = (at(arc.start)?, at(arc.end)?);
                    let angle = move |p: [f64; 2]| (p[1] - o[1]).atan2(p[0] - o[0]);
                    let (t0, t1) = (angle(s), angle(e));
                    let span = (t1 - t0).rem_euclid(std::f64::consts::TAU);
                    Some((
                        arc.id,
                        Box::new(move |p| {
                            if (angle(p) - t0).rem_euclid(std::f64::consts::TAU) <= span {
                                ((p[0] - o[0]).hypot(p[1] - o[1]) - r).abs()
                            } else {
                                (p[0] - s[0])
                                    .hypot(p[1] - s[1])
                                    .min((p[0] - e[0]).hypot(p[1] - e[1]))
                            }
                        }),
                    ))
                }
                _ => None,
            }
        })
        .collect()
}

fn to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = dx * dx + dy * dy;
    let t = if len > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}
