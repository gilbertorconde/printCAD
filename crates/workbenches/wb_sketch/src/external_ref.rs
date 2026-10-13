//! External geometry from another sketch or a datum: its curves, or the
//! datum's line, point or crossing with the sketch plane, brought onto this
//! sketch's plane. Worked out here rather than by the kernel: another
//! sketch's elements and a datum's frame are the document's own data.
//!
//! A curve in a plane parallel to this one keeps its shape (a circle stays
//! a circle); one at an angle comes as points along it, kept as a chain of
//! lines, as a projected edge with no closed form does.

use core_document::{DatumFeature, DatumShape, Document, FeatureId, WorkbenchFeature};
use glam::DVec3;

use crate::boundary::{place_direction, place_point, widen, widen3};
use kernel_api::ProjectedEdge;
use serde_json::Value;
use uuid::Uuid;

use crate::sketch::{ExternalReference, GeometryElement, SketchPlane};

/// Points along an arc or curve that is not kept whole.
const SAMPLES: usize = 48;

/// A plane's point `p` (world) in its own 2D coordinates.
fn to_2d(plane: &SketchPlane, p: DVec3) -> [f64; 2] {
    let d = p - DVec3::from_array(plane.origin);
    [
        d.dot(DVec3::from_array(plane.x_axis)),
        d.dot(DVec3::from_array(plane.y_axis)),
    ]
}

/// A plane's own 2D point in the world.
fn to_world(plane: &SketchPlane, x: f64, y: f64) -> DVec3 {
    DVec3::from_array(plane.origin)
        + DVec3::from_array(plane.x_axis) * x
        + DVec3::from_array(plane.y_axis) * y
}

/// What `reference` comes to on `plane` (the sketch's plane where it sits
/// in the world).
pub fn project(
    document: &Document,
    plane: &SketchPlane,
    reference: ExternalReference,
) -> Result<Vec<ProjectedEdge>, String> {
    project_with(
        document,
        &|id| document.feature_values(id).cloned(),
        plane,
        reference,
    )
}

/// [`project`], with `values_of` giving each sketch's and datum's working
/// data (a sketch's solved) where the document's last evaluation would.
pub fn project_with(
    document: &Document,
    values_of: &dyn Fn(FeatureId) -> Option<Value>,
    plane: &SketchPlane,
    reference: ExternalReference,
) -> Result<Vec<ProjectedEdge>, String> {
    match reference {
        ExternalReference::SketchElement { sketch, element } => {
            project_element(document, values_of, plane, FeatureId(sketch), element)
        }
        ExternalReference::Datum { datum } => {
            project_datum(document, values_of, plane, FeatureId(datum))
        }
    }
}

fn project_element(
    document: &Document,
    values_of: &dyn Fn(FeatureId) -> Option<Value>,
    plane: &SketchPlane,
    sketch: FeatureId,
    element: Uuid,
) -> Result<Vec<ProjectedEdge>, String> {
    let other = values_of(sketch)
        .and_then(|data| crate::SketchFeature::from_json(&data).ok())
        .ok_or("that sketch is gone")?;
    let from = crate::placed_plane(&other.plane, &crate::sketch_placement(document, sketch));
    let geom = other
        .sketch
        .get_geometry(element)
        .ok_or("that element is gone from its sketch")?;
    let at = |id: Uuid| -> Result<[f64; 2], String> {
        let p = other
            .sketch
            .point_position(id)
            .ok_or("an element's point is gone")?;
        Ok(to_2d(plane, to_world(&from, p.x, p.y)))
    };
    let normal = |p: &SketchPlane| DVec3::from_array(p.normal).normalize_or_zero();
    let facing = normal(&from).dot(normal(plane));
    let parallel = (facing.abs() - 1.0).abs() < 1e-5;
    let sampled = |points: Vec<crate::sketch::Vec2D>| {
        ProjectedEdge::Polyline(
            points
                .into_iter()
                .map(|p| to_2d(plane, to_world(&from, p.x, p.y)))
                .collect(),
        )
    };
    let edge = match geom {
        GeometryElement::Point(p) => {
            ProjectedEdge::Point(to_2d(plane, to_world(&from, p.position.x, p.position.y)))
        }
        GeometryElement::Line(l) => ProjectedEdge::Line {
            start: at(l.start)?,
            end: at(l.end)?,
        },
        GeometryElement::Circle(c) if parallel => ProjectedEdge::Circle {
            centre: at(c.center)?,
            radius: c.radius,
            range: (0.0, std::f64::consts::TAU),
        },
        GeometryElement::Arc(a) if parallel => {
            let centre = at(a.center)?;
            // Seen from the other side, counter-clockwise runs the other way.
            let (s, e) = if facing > 0.0 {
                (at(a.start)?, at(a.end)?)
            } else {
                (at(a.end)?, at(a.start)?)
            };
            let angle = |p: [f64; 2]| (p[1] - centre[1]).atan2(p[0] - centre[0]);
            let start = angle(s);
            let mut end = angle(e);
            while end <= start {
                end += std::f64::consts::TAU;
            }
            ProjectedEdge::Circle {
                centre,
                radius: a.radius,
                range: (start, end),
            }
        }
        _ => sampled(element_samples(&other.sketch, geom).ok_or("that curve cannot be read")?),
    };
    Ok(vec![edge])
}

/// Points along any curve of a sketch, in its own coordinates.
pub(crate) fn element_samples(
    sketch: &crate::sketch::Sketch,
    geom: &GeometryElement,
) -> Option<Vec<crate::sketch::Vec2D>> {
    use crate::sketch::Vec2D;
    let pos = |id: Uuid| sketch.point_position(id);
    let around = |c: Vec2D, point: &dyn Fn(f64) -> Vec2D, from: f64, to: f64| {
        (0..=SAMPLES)
            .map(|i| point(from + (to - from) * i as f64 / SAMPLES as f64))
            .map(|p| Vec2D::new(c.x + p.x, c.y + p.y))
            .collect::<Vec<_>>()
    };
    Some(match geom {
        GeometryElement::Circle(c) => {
            let r = c.radius;
            around(
                pos(c.center)?,
                &|t| Vec2D::new(r * t.cos(), r * t.sin()),
                0.0,
                std::f64::consts::TAU,
            )
        }
        GeometryElement::Arc(a) => {
            let c = pos(a.center)?;
            let (start, sweep) =
                crate::snap::arc_angles((pos(a.start)? - c).to_glam(), (pos(a.end)? - c).to_glam());
            let r = a.radius;
            around(
                c,
                &|t| Vec2D::new(r * t.cos(), r * t.sin()),
                start,
                start + sweep,
            )
        }
        GeometryElement::Ellipse(e) => {
            let c = pos(e.center)?;
            let m = e.major.to_glam();
            let w = m.perp() * e.ratio;
            let point = |t: f64| Vec2D::from_glam(m * t.cos() + w * t.sin());
            let (from, to) = match e.arc {
                Some(arc) => {
                    // The ends' angles in the ellipse's own frame.
                    let param = |p: Vec2D| {
                        let d = (p - c).to_glam();
                        let (u, v) = (d.dot(m) / m.length_squared(), d.dot(w) / w.length_squared());
                        v.atan2(u)
                    };
                    let (s, mut t) = (param(pos(arc.start)?), param(pos(arc.end)?));
                    while t <= s {
                        t += std::f64::consts::TAU;
                    }
                    (s, t)
                }
                None => (0.0, std::f64::consts::TAU),
            };
            around(c, &point, from, to)
        }
        GeometryElement::BSpline(_) | GeometryElement::Conic(_) => {
            crate::measure::curve_samples(sketch, geom.id())?
                .into_iter()
                .map(Vec2D::from_glam)
                .collect()
        }
        GeometryElement::Point(_) | GeometryElement::Line(_) => return None,
    })
}

fn project_datum(
    document: &Document,
    values_of: &dyn Fn(FeatureId) -> Option<Value>,
    plane: &SketchPlane,
    id: FeatureId,
) -> Result<Vec<ProjectedEdge>, String> {
    let data = values_of(id).ok_or("that datum is gone")?;
    let datum = DatumFeature::from_json(&data).map_err(|_| "that is not a datum".to_string())?;
    let placement = document
        .get_feature_meta(id)
        .and_then(|node| node.body)
        .map(|body| document.body_placement(body))
        .unwrap_or_default();
    let frame = datum.frame();
    let origin = DVec3::from_array(place_point(&placement, widen3(frame.origin)));
    let x =
        DVec3::from_array(place_direction(&placement, widen3(frame.x_axis))).normalize_or_zero();
    let normal =
        DVec3::from_array(place_direction(&placement, widen3(frame.normal))).normalize_or_zero();
    Ok(match datum.shape {
        DatumShape::Point | DatumShape::CoordinateSystem { .. } => {
            vec![ProjectedEdge::Point(to_2d(plane, origin))]
        }
        DatumShape::Line { length } => {
            let h = x * (widen(length) * 0.5);
            vec![ProjectedEdge::Line {
                start: to_2d(plane, origin - h),
                end: to_2d(plane, origin + h),
            }]
        }
        // Where the datum plane crosses this one, as long as the datum is
        // drawn, about the point of the crossing nearest the datum.
        DatumShape::Plane { size } => {
            let n = DVec3::from_array(plane.normal).normalize_or_zero();
            let along = normal.cross(n);
            if along.length() < 1e-5 {
                return Err("the datum plane is parallel to the sketch".to_string());
            }
            let along = along.normalize();
            // From the datum's origin, within the datum plane and square to
            // the crossing, to where the sketch plane is.
            let o = DVec3::from_array(plane.origin);
            let toward = along.cross(normal);
            let t = (o - origin).dot(n) / toward.dot(n);
            let meet = origin + toward * t;
            let h = along * (widen(size) * 0.5);
            vec![ProjectedEdge::Line {
                start: to_2d(plane, meet - h),
                end: to_2d(plane, meet + h),
            }]
        }
    })
}
