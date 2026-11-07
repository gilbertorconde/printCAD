//! Exact geometry projection for picking and drawing gestures.

use sketch_geometry::types::Angle;
use sketch_geometry::world::{WorldLength, WorldPerPx, WorldPoint, WorldVec};
use sketch_runtime::geometry::{PickElement, SourceId};
use sketch_runtime::pick::{Axis, PointRole, Source};
use sketch_runtime::scene::{ContactTolerance, PickScene};
use uuid::Uuid;

use crate::sketch::{GeometryElement, ORIGIN_ID, Sketch, Vec2D, X_AXIS_ID, Y_AXIS_ID};
use crate::snap::{self, Snap, SnapKind, SnapTarget};

#[derive(Debug, Default)]
pub(crate) struct SelectPick {
    sketch_id: Option<Uuid>,
    pub(crate) geometry: PickScene,
}

impl SelectPick {
    /// Constraints and history stay in the host document. Ellipses and conics
    /// use exact rational carriers; their representation poles stay private.
    pub(crate) fn sync(&mut self, sketch: &Sketch) {
        if self.sketch_id != Some(sketch.id) {
            self.geometry = PickScene::default();
            self.sketch_id = Some(sketch.id);
        }
        let input: Vec<_> = sketch
            .geometry
            .iter()
            .map(|g| match g {
                GeometryElement::Point(p) => PickElement::Point {
                    id: SourceId(p.id.as_u128()),
                    pos: WorldPoint::new(p.position.x, p.position.y),
                },
                GeometryElement::Line(l) => PickElement::Segment {
                    id: SourceId(l.id.as_u128()),
                    a: SourceId(l.start.as_u128()),
                    b: SourceId(l.end.as_u128()),
                },
                GeometryElement::Circle(c) => PickElement::Circle {
                    id: SourceId(c.id.as_u128()),
                    center: SourceId(c.center.as_u128()),
                    radius: WorldLength::new(c.radius),
                },
                GeometryElement::Arc(a) => {
                    let center = sketch.point_position(a.center).expect("arc centre");
                    let start = sketch.point_position(a.start).expect("arc start");
                    let end = sketch.point_position(a.end).expect("arc end");
                    let (start_angle, sweep) =
                        snap::arc_angles((start - center).to_glam(), (end - center).to_glam());
                    PickElement::Arc {
                        id: SourceId(a.id.as_u128()),
                        center: SourceId(a.center.as_u128()),
                        start: SourceId(a.start.as_u128()),
                        end: SourceId(a.end.as_u128()),
                        start_angle: Angle(start_angle),
                        sweep_angle: Angle(sweep),
                    }
                }
                GeometryElement::BSpline(b) => {
                    use sketch_geometry::bspline::{
                        SplineBasis, SplineDefinition, SplineDegree, SplineShape,
                    };
                    let basis = crate::spline::basis_of(b).expect("valid sketch spline");
                    PickElement::BSpline {
                        id: SourceId(b.id.as_u128()),
                        poles: b
                            .control_points
                            .iter()
                            .map(|id| SourceId(id.as_u128()))
                            .collect(),
                        shape: SplineShape {
                            definition: SplineDefinition::Poles,
                            degree: SplineDegree::new(basis.degree() as u32),
                            periodic: b.periodic,
                        },
                        basis: Some(SplineBasis {
                            knots: basis.knots().to_vec(),
                            weights: if basis.weights().is_empty() {
                                vec![1.; b.control_points.len()]
                            } else {
                                basis.weights().to_vec()
                            },
                        }),
                        span: None,
                    }
                }
                GeometryElement::Ellipse(e) => {
                    let center = sketch.point_position(e.center).expect("ellipse centre");
                    let (from, to) = e.param_span(sketch).expect("ellipse span");
                    let u = e.major.to_glam();
                    let (poles, basis) = sketch_geometry::analytic_spline::elliptic(
                        WorldPoint::new(center.x, center.y),
                        WorldVec::new(u.x, u.y),
                        WorldVec::new(-u.y * e.ratio, u.x * e.ratio),
                        from,
                        to - from,
                    )
                    .expect("finite ellipse frame");
                    PickElement::RationalCurve {
                        id: SourceId(e.id.as_u128()),
                        poles,
                        basis,
                        periodic: e.arc.is_none(),
                        endpoints: e
                            .arc
                            .map(|a| [SourceId(a.start.as_u128()), SourceId(a.end.as_u128())]),
                        center: Some(SourceId(e.center.as_u128())),
                    }
                }
                GeometryElement::Conic(c) => {
                    let (shape, a, b) = c.params(sketch).expect("finite conic frame");
                    let (poles, w) = shape.quadratic(a, b);
                    PickElement::RationalCurve {
                        id: SourceId(c.id.as_u128()),
                        poles: poles
                            .into_iter()
                            .map(|[x, y]| WorldPoint::new(x, y))
                            .collect(),
                        basis: sketch_geometry::bspline::SplineBasis {
                            knots: vec![0., 0., 0., 1., 1., 1.],
                            weights: vec![1., w, 1.],
                        },
                        periodic: false,
                        endpoints: Some([SourceId(c.start.as_u128()), SourceId(c.end.as_u128())]),
                        center: Some(SourceId(c.center.as_u128())),
                    }
                }
            })
            .collect();
        self.geometry.sync(input);
    }

    pub(crate) fn contact_tolerance() -> ContactTolerance {
        let tolerance = WorldLength::new(1e-6);
        ContactTolerance {
            distance: tolerance,
            minimum_segment_length: tolerance,
        }
    }

    pub(crate) fn source_uuid(source: Source) -> Uuid {
        match source {
            Source::Geometry(id) => Uuid::from_u128(id.0),
            Source::Origin => ORIGIN_ID,
            Source::Axis(Axis::Horizontal) => X_AXIS_ID,
            Source::Axis(Axis::Vertical) => Y_AXIS_ID,
            Source::Guide => panic!("a tool guide has no document identity"),
        }
    }

    pub fn snap(&mut self, sketch: &Sketch, cursor: Vec2D, scale: f64) -> Snap {
        self.sync(sketch);
        let Some(hit) = self.geometry.snap_at(
            WorldPoint::new(cursor.x, cursor.y),
            WorldPerPx::new(scale),
            Self::contact_tolerance(),
        ) else {
            return Snap::free(cursor);
        };
        self.host_geometric_snap(&hit)
    }

    pub(crate) fn host_geometric_snap(&self, hit: &sketch_runtime::snap::SnapResult) -> Snap {
        let pos = Vec2D::new(hit.snapped_pos.0.x, hit.snapped_pos.0.y);
        let point = self.geometry.point_source(hit.source);
        let kind = if hit.source.source == Source::Origin {
            SnapKind::Origin
        } else {
            match hit.kind {
                sketch_runtime::pick::SnapKind::Endpoint => SnapKind::Endpoint,
                sketch_runtime::pick::SnapKind::Center => SnapKind::Center,
                sketch_runtime::pick::SnapKind::Quadrant => {
                    let PointRole::Quadrant(q) = hit.source.role else {
                        unreachable!("quadrant snap must carry its quadrant point role")
                    };
                    SnapKind::Quadrant(q)
                }
                sketch_runtime::pick::SnapKind::Midpoint => SnapKind::Midpoint,
                sketch_runtime::pick::SnapKind::Intersection => SnapKind::Intersection,
                sketch_runtime::pick::SnapKind::OnCurve => SnapKind::OnCurve,
                sketch_runtime::pick::SnapKind::OnAxis => SnapKind::OnAxis,
                sketch_runtime::pick::SnapKind::Parallel => SnapKind::Parallel,
                sketch_runtime::pick::SnapKind::Perpendicular => SnapKind::Perpendicular,
                sketch_runtime::pick::SnapKind::PerpendicularTo => SnapKind::PerpendicularTo,
                sketch_runtime::pick::SnapKind::OnExtension => SnapKind::OnExtension,
                sketch_runtime::pick::SnapKind::Tangent => SnapKind::Tangent,
            }
        };
        Snap {
            target: point.map_or(SnapTarget::New(pos), |key| {
                SnapTarget::Existing(Uuid::from_u128(key.0))
            }),
            pos,
            kind: Some(kind),
            curve: point
                .is_none()
                .then(|| Self::source_uuid(hit.source.source)),
            curve2: hit.source2.map(|source| Self::source_uuid(source.source)),
            on: matches!(
                hit.kind,
                sketch_runtime::pick::SnapKind::Parallel
                    | sketch_runtime::pick::SnapKind::PerpendicularTo
            )
            .then(|| hit.source2.map(|s| Self::landing_kind(s.source)))
            .flatten(),
        }
    }

    pub(crate) fn landing_kind(source: Source) -> SnapKind {
        match source {
            Source::Axis(_) => SnapKind::OnAxis,
            Source::Geometry(_) => SnapKind::OnCurve,
            _ => unreachable!("direction endpoint names a curve or axis"),
        }
    }

    pub(crate) fn hits(&mut self, sketch: &Sketch, cursor: Vec2D, scale: f64) -> Vec<Uuid> {
        self.sync(sketch);
        let tolerance = sketch_runtime::hover::hover_tolerance(WorldPerPx::new(scale));
        let hair = (tolerance.0 * 1e-3).max(f64::MIN_POSITIVE);
        let mut hits = self
            .geometry
            .hover_candidates(WorldPoint::new(cursor.x, cursor.y), tolerance);
        hits.sort_by_key(|hit| {
            let id = Self::source_uuid(hit.geo);
            let index = sketch.geometry.iter().position(|g| g.id() == id);
            (
                index.is_none(),
                hit.rank,
                sketch.is_construction(id),
                (hit.distance.0 / hair).round() as i64,
                std::cmp::Reverse(index),
                id,
            )
        });
        hits.into_iter()
            .map(|hit| Self::source_uuid(hit.geo))
            .collect()
    }

    pub fn hover(&mut self, sketch: &Sketch, cursor: Vec2D, scale: f64) -> Option<Uuid> {
        self.hits(sketch, cursor, scale).first().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Line, Point};

    #[test]
    fn selected_sources_follow_uuids_across_reordering_moves_and_reload() {
        let mut sketch = Sketch::new("pick");
        let mut points = Vec::new();
        for (x, y) in [(20., 10.), (40., 30.), (20., 30.), (40., 10.)] {
            points.push(sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y)))));
        }
        let a = sketch.add_geometry(GeometryElement::Line(Line::new(points[0], points[1])));
        let b = sketch.add_geometry(GeometryElement::Line(Line::new(points[2], points[3])));
        let mut picker = SelectPick::default();
        let at = Vec2D::new(30.1, 20.);
        for reload in [false, true] {
            let hit = picker.snap(&sketch, at, 0.05);
            assert_eq!(hit.kind, Some(SnapKind::Intersection));
            assert_eq!(
                [hit.curve.unwrap(), hit.curve2.unwrap()],
                [a.min(b), a.max(b)]
            );
            assert_eq!(hit.pos, Vec2D::new(30., 20.));
            sketch.geometry.reverse();
            if reload {
                sketch = serde_json::from_value(serde_json::to_value(&sketch).unwrap()).unwrap();
                picker = SelectPick::default();
            }
        }
        let endpoint = picker.snap(&sketch, Vec2D::new(20.1, 10.), 0.05);
        assert_eq!(endpoint.target, SnapTarget::Existing(points[0]));
        for g in &mut sketch.geometry {
            if let GeometryElement::Point(p) = g {
                p.position.y += 5.;
            }
        }
        let hit = picker.snap(&sketch, Vec2D::new(30.1, 25.), 0.05);
        assert_eq!(hit.kind, Some(SnapKind::Intersection));
        assert_eq!(hit.pos, Vec2D::new(30., 25.));
        assert!(picker.snap(&sketch, at, 0.05).kind.is_none());
        let saved = sketch.geometry.clone();
        sketch.geometry.clear();
        assert!(
            picker
                .snap(&sketch, Vec2D::new(30.1, 25.), 0.05)
                .kind
                .is_none()
        );
        sketch.geometry = saved;
        assert_eq!(
            picker.snap(&sketch, Vec2D::new(30.1, 25.), 0.05).kind,
            Some(SnapKind::Intersection)
        );
    }
}
