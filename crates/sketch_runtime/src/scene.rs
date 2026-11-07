//! Incremental geometric projection. Sources belong to the host; this scene
//! contains no document, solver, history or application of drawing effects.

use std::collections::{BTreeMap, BTreeSet};

use sketch_geometry::arc::ArcGeom;
use sketch_geometry::bspline::{BSplineCurve, SplineDefinition, SplineDegree, SplineShape};
use sketch_geometry::bspline_window::SplineWindow;
use sketch_geometry::conic::ConicArcParams;
use sketch_geometry::profile_curve::ProfileCurve;
use sketch_geometry::trim_curve::TrimCurve;
use sketch_geometry::world::{WorldLength, WorldPerPx, WorldPoint, WorldVec};

use crate::geometry::{PickElement, SourceId};
use crate::hover::{HoverCandidate, HoverGeom, HoverObj};
use crate::intersection::ContactCurve;
use crate::pick::{Axis, PointRole, SnapKind, Source, SourceRef};
use crate::snap::{SnapGeom, SnapObj, SnapResult};

/// Model tolerances are supplied by the host, independently of pointer reach.
#[derive(Debug, Clone, Copy)]
pub struct ContactTolerance {
    pub distance: WorldLength,
    pub minimum_segment_length: WorldLength,
}

#[derive(Debug, Default)]
struct Resolved {
    snaps: Vec<SnapObj>,
    hover: Vec<HoverObj>,
    contact: Option<ContactCurve<SourceRef>>,
    emitted: BTreeSet<SourceId>,
}

#[derive(Debug, Default)]
pub struct PickScene {
    elements: BTreeMap<SourceId, PickElement>,
    resolved: BTreeMap<SourceId, Resolved>,
}

impl PickScene {
    /// Exact topology reuse, with stable source order when several points coincide.
    pub fn point_at(&self, pos: WorldPoint) -> Option<SourceId> {
        self.elements.values().find_map(|element| match element {
            PickElement::Point { id, pos: at } if at.distance(pos) <= WorldLength::DEGENERATE => {
                Some(*id)
            }
            _ => None,
        })
    }
    /// Exact live input, addressed by the host's stable identity.
    pub fn element(&self, id: SourceId) -> Option<&PickElement> {
        self.elements.get(&id)
    }
    pub fn point_position(&self, id: SourceId) -> Option<WorldPoint> {
        match self.elements.get(&id) {
            Some(PickElement::Point { pos, .. }) => Some(*pos),
            _ => None,
        }
    }

    pub fn direction_objects(&self, query: crate::direction::DirectionQuery) -> Vec<SnapObj> {
        use crate::direction::DirectionQuery;
        match query {
            DirectionQuery::None => Vec::new(),
            DirectionQuery::Extensions => crate::direction::extension_snaps(&self.snap_objects()),
            DirectionQuery::From { pos, vertex } => {
                let objects = self.snap_objects();
                let directions = crate::direction::direction_snaps(&objects, pos, |source| {
                    let Source::Geometry(id) = source.source else {
                        return false;
                    };
                    matches!(self.elements.get(&id), Some(PickElement::Segment { a, b, .. })
                        if vertex.is_some_and(|p| p == *a || p == *b))
                });
                crate::direction::line_snaps(pos, at(Source::Guide, PointRole::Edge), directions)
            }
        }
    }

    /// Rebuild only changed carriers and carriers whose referenced points moved.
    /// Removal and restoration retain the host's addresses without tombstones.
    pub fn sync(&mut self, elements: impl IntoIterator<Item = PickElement>) {
        let mut next = BTreeMap::new();
        for element in elements {
            assert!(
                next.insert(element.id(), element).is_none(),
                "duplicate projected source"
            );
        }
        let changed: BTreeSet<_> = next
            .iter()
            .filter_map(|(id, element)| (self.elements.get(id) != Some(element)).then_some(*id))
            .chain(
                self.elements
                    .keys()
                    .filter(|id| !next.contains_key(id))
                    .copied(),
            )
            .collect();
        self.resolved.retain(|id, _| next.contains_key(id));
        for (id, element) in &next {
            let refs = references(element);
            // Validate unchanged carriers as well: deleting a live endpoint is
            // a host projection error, not a reason to retain a stale curve.
            for point in &refs {
                position(&next, *point);
            }
            if changed.contains(id) || refs.iter().any(|p| changed.contains(p)) {
                self.resolved.insert(*id, resolve(element, &next));
            }
        }
        self.elements = next;
    }

    /// Resolve topology by address, never by coordinate coincidence.
    pub fn point_source(&self, source: SourceRef) -> Option<SourceId> {
        let Source::Geometry(id) = source.source else {
            return None;
        };
        match (self.elements.get(&id)?, source.role) {
            (PickElement::Point { .. }, _) => Some(id),
            (PickElement::Segment { a, .. }, PointRole::Start) => Some(*a),
            (PickElement::Segment { b, .. }, PointRole::End) => Some(*b),
            (
                PickElement::Circle { center, .. } | PickElement::Ellipse { center, .. },
                PointRole::Center,
            ) => Some(*center),
            (
                PickElement::Arc { center, .. }
                | PickElement::EllipseArc { center, .. }
                | PickElement::HyperbolaArc { center, .. },
                PointRole::Center,
            ) => Some(*center),
            (PickElement::ParabolaArc { vertex, .. }, PointRole::Center) => Some(*vertex),
            (
                PickElement::Arc { start, .. }
                | PickElement::EllipseArc { start, .. }
                | PickElement::HyperbolaArc { start, .. }
                | PickElement::ParabolaArc { start, .. },
                PointRole::Start,
            ) => Some(*start),
            (
                PickElement::Arc { end, .. }
                | PickElement::EllipseArc { end, .. }
                | PickElement::HyperbolaArc { end, .. }
                | PickElement::ParabolaArc { end, .. },
                PointRole::End,
            ) => Some(*end),
            (
                PickElement::BSpline {
                    poles, shape, span, ..
                },
                role @ (PointRole::Start | PointRole::End),
            ) => {
                let ends = span
                    .map(|s| s.endpoints)
                    .or_else(|| (!shape.periodic).then(|| [poles[0], *poles.last().unwrap()]))?;
                Some(ends[usize::from(role == PointRole::End)])
            }
            (
                PickElement::RationalCurve {
                    endpoints: Some(ends),
                    ..
                },
                role @ (PointRole::Start | PointRole::End),
            ) => Some(ends[usize::from(role == PointRole::End)]),
            (PickElement::RationalCurve { center, .. }, PointRole::Center) => *center,
            _ => None,
        }
    }

    pub fn snap_objects(&self) -> Vec<SnapObj> {
        let emitted: BTreeSet<_> = self
            .resolved
            .values()
            .flat_map(|r| r.emitted.iter().copied())
            .collect();
        let centers: BTreeSet<_> = self
            .elements
            .values()
            .filter_map(|element| match element {
                PickElement::RationalCurve { center, .. } => *center,
                _ => None,
            })
            .collect();
        let mut objects: Vec<_> = self
            .resolved
            .iter()
            .filter(|(id, _)| {
                !matches!(self.elements[id], PickElement::Point { .. }) || !emitted.contains(id)
            })
            .flat_map(|(id, r)| {
                r.snaps.iter().cloned().map(|mut object| {
                    if matches!(self.elements[id], PickElement::Point { .. })
                        && centers.contains(id)
                    {
                        object.source.role = PointRole::Center;
                    }
                    object
                })
            })
            .collect();
        objects.extend([
            SnapObj {
                geom: SnapGeom::Point(WorldPoint::ORIGIN),
                source: at(Source::Origin, PointRole::Edge),
            },
            SnapObj {
                geom: SnapGeom::Line(WorldPoint::ORIGIN, WorldPoint::new(1., 0.)),
                source: at(Source::Axis(Axis::Horizontal), PointRole::Edge),
            },
            SnapObj {
                geom: SnapGeom::Line(WorldPoint::ORIGIN, WorldPoint::new(0., 1.)),
                source: at(Source::Axis(Axis::Vertical), PointRole::Edge),
            },
        ]);
        objects
    }

    pub fn hover_candidates(
        &self,
        cursor: WorldPoint,
        tolerance: WorldLength,
    ) -> Vec<HoverCandidate> {
        let mut objects: Vec<_> = self
            .resolved
            .values()
            .flat_map(|r| r.hover.iter().cloned())
            .collect();
        objects.extend([
            HoverObj {
                geo: Source::Origin,
                geom: HoverGeom::Origin,
            },
            HoverObj {
                geo: Source::Axis(Axis::Horizontal),
                geom: HoverGeom::HAxis,
            },
            HoverObj {
                geo: Source::Axis(Axis::Vertical),
                geom: HoverGeom::VAxis,
            },
        ]);
        crate::hover::hover_candidates_in_tolerance(&objects, cursor, tolerance)
    }

    pub fn hover_at(&self, cursor: WorldPoint, scale: WorldPerPx) -> Option<Source> {
        self.hover_candidates(cursor, crate::hover::hover_tolerance(scale))
            .into_iter()
            .min_by(crate::hover::hover_cmp)
            .map(|c| c.geo)
    }

    pub fn intersections(
        &self,
        cursor: WorldPoint,
        tolerance: WorldLength,
        contact: ContactTolerance,
    ) -> Vec<SnapResult> {
        let mut candidates = crate::intersection::collect_intersection_snaps(
            &self.snap_objects(),
            cursor,
            tolerance,
        );
        let curves: Vec<_> = self
            .resolved
            .values()
            .filter_map(|r| r.contact.clone())
            .collect();
        candidates.extend(
            crate::intersection::rational_intersections_near(
                &curves,
                cursor,
                tolerance,
                contact.distance,
                contact.minimum_segment_length,
            )
            .into_iter()
            .map(|(point, a, b)| SnapResult {
                snapped_pos: point,
                distance: point.distance(cursor),
                kind: SnapKind::Intersection,
                priority: 2,
                source: a,
                source2: Some(b),
            }),
        );
        candidates
    }

    pub fn snap_candidates(
        &self,
        cursor: WorldPoint,
        tolerance: WorldLength,
        extra: &[SnapObj],
        contact: ContactTolerance,
    ) -> Vec<SnapResult> {
        let mut objects = self.snap_objects();
        objects.extend_from_slice(extra);
        let mut candidates = crate::snap::candidates_in_tolerance(cursor, &objects, tolerance);
        candidates.extend(self.intersections(cursor, tolerance, contact));
        for candidate in &mut candidates {
            candidate.canonicalize_sources(|s| s);
        }
        candidates
    }

    pub fn snap_at(
        &self,
        cursor: WorldPoint,
        scale: WorldPerPx,
        contact: ContactTolerance,
    ) -> Option<SnapResult> {
        self.snap_candidates(cursor, crate::snap::snap_tolerance(scale), &[], contact)
            .into_iter()
            .min_by(snap_cmp)
    }
}

/// Synthetic gesture axes use the same final address rank as the horizontal
/// reference axis, preserving the existing placement tie-break.
pub fn snap_cmp(a: &SnapResult, b: &SnapResult) -> std::cmp::Ordering {
    crate::snap::snap_cmp_by_key(a, b, |source| {
        let address = match source.source {
            Source::Guide => Source::Axis(Axis::Horizontal),
            source => source,
        };
        (address, source.role)
    })
}

fn at(source: Source, role: PointRole) -> SourceRef {
    SourceRef { source, role }
}

fn position(elements: &BTreeMap<SourceId, PickElement>, id: SourceId) -> WorldPoint {
    match elements.get(&id) {
        Some(PickElement::Point { pos, .. }) => *pos,
        _ => panic!("projected endpoint is not an active point: {id:?}"),
    }
}

fn references(element: &PickElement) -> Vec<SourceId> {
    match element {
        PickElement::Point { .. } => vec![],
        PickElement::Segment { a, b, .. } => vec![*a, *b],
        PickElement::Circle { center, .. } | PickElement::Ellipse { center, .. } => vec![*center],
        PickElement::Arc {
            center, start, end, ..
        }
        | PickElement::EllipseArc {
            center, start, end, ..
        } => vec![*center, *start, *end],
        PickElement::HyperbolaArc {
            center,
            focus1,
            start,
            end,
            ..
        } => vec![*center, *focus1, *start, *end],
        PickElement::ParabolaArc {
            vertex,
            focus,
            start,
            end,
            ..
        } => vec![*vertex, *focus, *start, *end],
        PickElement::BSpline { poles, span, .. } => poles
            .iter()
            .copied()
            .chain(span.iter().flat_map(|s| s.endpoints))
            .collect(),
        PickElement::RationalCurve {
            endpoints, center, ..
        } => endpoints.iter().flatten().copied().chain(*center).collect(),
    }
}

impl Resolved {
    fn point(&mut self, source: SourceRef, pos: WorldPoint, point: Option<SourceId>) {
        self.snaps.push(SnapObj {
            source,
            geom: SnapGeom::Point(pos),
        });
        self.emitted.extend(point);
    }
    fn curve(
        &mut self,
        source: SourceRef,
        snap: SnapGeom,
        hover: HoverGeom,
        curve: ProfileCurve,
        ends: Option<[WorldPoint; 2]>,
    ) {
        self.snaps.push(SnapObj { source, geom: snap });
        self.hover.push(HoverObj {
            geo: source.source,
            geom: hover,
        });
        self.contact = Some(ContactCurve {
            source,
            curve,
            ends,
        });
    }
}

fn rational(
    poles: &[WorldPoint],
    basis: &sketch_geometry::bspline::SplineBasis,
    periodic: bool,
) -> SplineWindow {
    let shape = SplineShape {
        definition: SplineDefinition::Poles,
        degree: SplineDegree::new(2),
        periodic,
    };
    SplineWindow::whole(
        BSplineCurve::build_with_basis(shape, Some(basis), poles)
            .expect("valid exact rational carrier"),
    )
}

fn resolve(element: &PickElement, elements: &BTreeMap<SourceId, PickElement>) -> Resolved {
    let mut out = Resolved::default();
    let source = Source::Geometry(element.id());
    let edge = at(source, PointRole::Edge);
    let p = |id| position(elements, id);
    match element {
        PickElement::Point { pos, .. } => {
            out.point(edge, *pos, None);
            out.hover.push(HoverObj {
                geo: source,
                geom: HoverGeom::Point(*pos),
            });
        }
        PickElement::Segment { a, b, .. } => {
            let (start, end) = (p(*a), p(*b));
            out.point(at(source, PointRole::Start), start, Some(*a));
            out.point(at(source, PointRole::End), end, Some(*b));
            out.point(at(source, PointRole::Mid), start.midpoint(end), None);
            out.curve(
                edge,
                SnapGeom::Segment(start, end),
                HoverGeom::Segment(start, end),
                ProfileCurve::Analytic(TrimCurve::Segment { a: start, b: end }),
                Some([start, end]),
            );
        }
        PickElement::Circle { center, radius, .. } => {
            let c = p(*center);
            out.point(at(source, PointRole::Center), c, Some(*center));
            for (q, offset) in [
                (radius.0, 0.),
                (0., radius.0),
                (-radius.0, 0.),
                (0., -radius.0),
            ]
            .into_iter()
            .enumerate()
            {
                out.point(
                    at(source, PointRole::Quadrant(q as u8)),
                    c + WorldVec::new(offset.0, offset.1),
                    None,
                );
            }
            out.curve(
                edge,
                SnapGeom::Circle {
                    center: c,
                    radius: *radius,
                },
                HoverGeom::Circle {
                    center: c,
                    radius: *radius,
                },
                ProfileCurve::Analytic(TrimCurve::Circle {
                    center: c,
                    radius: *radius,
                }),
                None,
            );
        }
        PickElement::Arc {
            center,
            start,
            end,
            start_angle,
            sweep_angle,
            ..
        } => {
            let (c, a, b) = (p(*center), p(*start), p(*end));
            let radius = c.distance(a);
            for (role, pos, id) in [
                (PointRole::Center, c, *center),
                (PointRole::Start, a, *start),
                (PointRole::End, b, *end),
            ] {
                out.point(at(source, role), pos, Some(id));
            }
            out.curve(
                edge,
                SnapGeom::Arc {
                    center: c,
                    radius,
                    start_angle: *start_angle,
                    sweep_angle: *sweep_angle,
                },
                HoverGeom::Arc {
                    center: c,
                    start: a,
                    end: b,
                    start_angle: *start_angle,
                    sweep_angle: *sweep_angle,
                },
                ProfileCurve::Analytic(TrimCurve::Arc {
                    center: c,
                    radius,
                    geom: ArcGeom {
                        start_angle: *start_angle,
                        sweep_angle: *sweep_angle,
                    },
                }),
                Some([a, b]),
            );
        }
        PickElement::Ellipse {
            center,
            focus1,
            start,
            ..
        } => {
            let c = p(*center);
            out.point(at(source, PointRole::Center), c, Some(*center));
            conic(
                &mut out,
                edge,
                ConicArcParams::ellipse(c, *focus1, *start, *start, false),
                *start,
                *start,
                HoverGeom::EllipseArc {
                    center: c,
                    focus1: *focus1,
                    start: *start,
                    end: *start,
                    clockwise: false,
                },
                true,
            );
        }
        PickElement::EllipseArc {
            center,
            focus1,
            start,
            end,
            clockwise,
            ..
        } => {
            let (c, a, b) = (p(*center), p(*start), p(*end));
            for (role, pos, id) in [
                (PointRole::Center, c, *center),
                (PointRole::Start, a, *start),
                (PointRole::End, b, *end),
            ] {
                out.point(at(source, role), pos, Some(id));
            }
            conic(
                &mut out,
                edge,
                ConicArcParams::ellipse(c, *focus1, a, b, *clockwise),
                a,
                b,
                HoverGeom::EllipseArc {
                    center: c,
                    focus1: *focus1,
                    start: a,
                    end: b,
                    clockwise: *clockwise,
                },
                false,
            );
        }
        PickElement::HyperbolaArc {
            center,
            focus1,
            start,
            end,
            ..
        } => {
            let (c, f, a, b) = (p(*center), p(*focus1), p(*start), p(*end));
            let Some(params) = ConicArcParams::hyperbola(c, f, a, b) else {
                return out;
            };
            for (role, pos, id) in [
                (PointRole::Center, c, *center),
                (PointRole::Start, a, *start),
                (PointRole::End, b, *end),
            ] {
                out.point(at(source, role), pos, Some(id));
            }
            conic(
                &mut out,
                edge,
                params,
                a,
                b,
                HoverGeom::HyperbolaArc {
                    center: c,
                    focus1: f,
                    start: a,
                    end: b,
                },
                false,
            );
        }
        PickElement::ParabolaArc {
            vertex,
            focus,
            start,
            end,
            ..
        } => {
            let (v, f, a, b) = (p(*vertex), p(*focus), p(*start), p(*end));
            for (role, pos, id) in [
                (PointRole::Center, v, *vertex),
                (PointRole::Start, a, *start),
                (PointRole::End, b, *end),
            ] {
                out.point(at(source, role), pos, Some(id));
            }
            conic(
                &mut out,
                edge,
                ConicArcParams::parabola(v, f, a, b),
                a,
                b,
                HoverGeom::ParabolaArc {
                    vertex: v,
                    focus: f,
                    start: a,
                    end: b,
                },
                false,
            );
        }
        PickElement::BSpline {
            poles,
            shape,
            basis,
            span,
            ..
        } => {
            let positions: Vec<_> = poles.iter().map(|id| p(*id)).collect();
            // A collapsed spline retains its model point targets but has no curve.
            let Ok(curve) = BSplineCurve::build_with_basis(*shape, basis.as_ref(), &positions)
            else {
                return out;
            };
            let window = span.map_or_else(
                || SplineWindow::whole(curve.clone()),
                |s| {
                    SplineWindow::on_window(
                        curve.clone(),
                        sketch_geometry::bspline_window::directed_window(&curve, s.parameters).0,
                    )
                },
            );
            let ends = span
                .map(|s| s.endpoints)
                .or_else(|| (!shape.periodic).then(|| [poles[0], *poles.last().unwrap()]));
            if let Some([a, b]) = ends {
                out.point(at(source, PointRole::Start), p(a), Some(a));
                out.point(at(source, PointRole::End), p(b), Some(b));
            }
            out.curve(
                edge,
                SnapGeom::BSpline(window.clone()),
                HoverGeom::BSpline(window.clone()),
                ProfileCurve::Spline(window),
                ends.map(|ends| ends.map(p)),
            );
        }
        PickElement::RationalCurve {
            poles,
            basis,
            periodic,
            endpoints,
            ..
        } => {
            let window = rational(poles, basis, *periodic);
            if let Some([a, b]) = endpoints {
                out.point(at(source, PointRole::Start), p(*a), Some(*a));
                out.point(at(source, PointRole::End), p(*b), Some(*b));
            }
            let ends = endpoints
                .map(|ends| ends.map(p))
                .or_else(|| (!window.closed()).then(|| window.window().map(|t| window.point(t))));
            out.curve(
                edge,
                SnapGeom::BSpline(window.clone()),
                HoverGeom::BSpline(window.clone()),
                ProfileCurve::Spline(window),
                ends,
            );
        }
    }
    out
}

fn conic(
    out: &mut Resolved,
    edge: SourceRef,
    params: ConicArcParams,
    start: WorldPoint,
    end: WorldPoint,
    hover: HoverGeom,
    closed: bool,
) {
    let closed = closed
        || matches!(params, ConicArcParams::Ellipse { t_start, t_end, clockwise, .. }
        if sketch_geometry::curves::ellipse_arc_sweep(t_start, t_end, clockwise).0.abs() == std::f64::consts::TAU);
    let snap = match params {
        ConicArcParams::Ellipse {
            center,
            focus1,
            semi_minor,
            t_start,
            t_end,
            clockwise,
        } => SnapGeom::ArcOfEllipse {
            center,
            focus1,
            semi_minor,
            t_start,
            t_end,
            clockwise,
        },
        ConicArcParams::Hyperbola {
            center,
            focus1,
            semi_minor,
            t_start,
            t_end,
            branch,
        } => SnapGeom::ArcOfHyperbola {
            center,
            focus1,
            semi_minor,
            t_start,
            t_end,
            branch,
        },
        ConicArcParams::Parabola {
            vertex,
            focus,
            u_start,
            u_end,
        } => SnapGeom::ArcOfParabola {
            vertex,
            focus,
            u_start,
            u_end,
        },
    };
    out.snaps.push(SnapObj {
        source: edge,
        geom: snap,
    });
    out.hover.push(HoverObj {
        geo: edge.source,
        geom: hover,
    });
    // Degenerate native conics can remain in old documents. As in curve editing,
    // only their valid exact carriers participate in refined intersections.
    if let Ok((poles, basis)) = sketch_geometry::analytic_spline::conic(params, start, end) {
        out.contact = Some(ContactCurve {
            source: edge,
            curve: ProfileCurve::Spline(rational(&poles, &basis, closed)),
            ends: (!closed).then_some([start, end]),
        });
    }
}
