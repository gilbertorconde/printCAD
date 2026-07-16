//! Extraction of closed profile wires from sketch geometry, for
//! consumption by solid-modeling features (pad/pocket).
//!
//! Curves are stitched into loops through their *shared point ids* — the
//! sketcher's endpoint snapping reuses point elements, so a visually closed
//! profile is topologically closed here with no coincidence tolerance.

use std::collections::HashMap;

use kernel_api::{ProfilePlane, ProfileSegment, ProfileWire};
use uuid::Uuid;

use crate::sketch::{GeometryElement, Sketch, SketchPlane, Vec2D};
use crate::snap::arc_angles;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    /// The sketch has no closed geometry to extrude.
    Empty,
    /// A curve endpoint is used by only one curve — the loop never closes.
    OpenAt(Uuid),
    /// More than two curves meet at one point; the loop is ambiguous.
    BranchingAt(Uuid),
    /// A curve references a missing point element.
    MissingPoint(Uuid),
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileError::Empty => write!(f, "sketch contains no closed profile"),
            ProfileError::OpenAt(_) => write!(f, "profile is not closed (open endpoint)"),
            ProfileError::BranchingAt(_) => {
                write!(f, "profile branches (more than two curves share a point)")
            }
            ProfileError::MissingPoint(_) => write!(f, "curve references a missing point"),
        }
    }
}

fn v2(p: Vec2D) -> [f64; 2] {
    [p.x as f64, p.y as f64]
}

/// Mid-point of the CCW arc from `start` to `end` around `center`.
fn arc_midpoint(center: Vec2D, start: Vec2D, end: Vec2D) -> Vec2D {
    let sv = (start - center).to_glam();
    let ev = (end - center).to_glam();
    let radius = sv.length();
    let (start_angle, sweep) = arc_angles(sv, ev);
    let mid_angle = start_angle + sweep * 0.5;
    Vec2D::new(
        center.x + radius * mid_angle.cos(),
        center.y + radius * mid_angle.sin(),
    )
}

/// A curve edge in the endpoint graph.
struct EdgeCurve {
    /// Endpoint point ids (start, end).
    ends: (Uuid, Uuid),
    segment: ProfileSegment,
}

/// Convert the world-space sketch plane into the kernel's profile plane.
pub fn plane_of(plane: &SketchPlane) -> ProfilePlane {
    ProfilePlane {
        origin: plane.origin.map(f64::from),
        x_axis: plane.x_axis.map(f64::from),
        y_axis: plane.y_axis.map(f64::from),
        normal: plane.normal.map(f64::from),
    }
}

/// Extract every closed wire from the sketch. Standalone points are
/// ignored; circles are closed wires by themselves; lines/arcs must form
/// closed loops via shared endpoints.
pub fn extract_wires(sketch: &Sketch) -> Result<Vec<ProfileWire>, ProfileError> {
    let mut wires = Vec::new();
    let mut edges: Vec<EdgeCurve> = Vec::new();

    for geom in &sketch.geometry {
        // Construction geometry is a drawing guide, never part of the
        // profile.
        if sketch.is_construction(geom.id()) {
            continue;
        }
        match geom {
            GeometryElement::Point(_) => {}
            GeometryElement::Circle(c) => {
                let center = sketch
                    .point_position(c.center)
                    .ok_or(ProfileError::MissingPoint(c.center))?;
                wires.push(ProfileWire {
                    segments: vec![ProfileSegment::Circle {
                        center: v2(center),
                        radius: c.radius as f64,
                    }],
                });
            }
            GeometryElement::Line(l) => {
                let a = sketch
                    .point_position(l.start)
                    .ok_or(ProfileError::MissingPoint(l.start))?;
                let b = sketch
                    .point_position(l.end)
                    .ok_or(ProfileError::MissingPoint(l.end))?;
                edges.push(EdgeCurve {
                    ends: (l.start, l.end),
                    segment: ProfileSegment::Line {
                        start: v2(a),
                        end: v2(b),
                    },
                });
            }
            GeometryElement::Arc(arc) => {
                let c = sketch
                    .point_position(arc.center)
                    .ok_or(ProfileError::MissingPoint(arc.center))?;
                let s = sketch
                    .point_position(arc.start)
                    .ok_or(ProfileError::MissingPoint(arc.start))?;
                let e = sketch
                    .point_position(arc.end)
                    .ok_or(ProfileError::MissingPoint(arc.end))?;
                edges.push(EdgeCurve {
                    ends: (arc.start, arc.end),
                    segment: ProfileSegment::Arc {
                        start: v2(s),
                        mid: v2(arc_midpoint(c, s, e)),
                        end: v2(e),
                    },
                });
            }
            // A full ellipse is a closed wire by itself, like a circle.
            GeometryElement::Ellipse(e) => {
                let center = sketch
                    .point_position(e.center)
                    .ok_or(ProfileError::MissingPoint(e.center))?;
                wires.push(ProfileWire {
                    segments: vec![ProfileSegment::Ellipse {
                        center: v2(center),
                        major: [f64::from(e.major.x), f64::from(e.major.y)],
                        ratio: f64::from(e.ratio),
                    }],
                });
            }
            // Periodic B-splines close on themselves; open ones connect via
            // their first/last control point like any other curve.
            GeometryElement::BSpline(b) => {
                if b.control_points.len() < 2 {
                    continue; // degenerate: nothing to contribute
                }
                let control_points = b
                    .control_points
                    .iter()
                    .map(|id| {
                        sketch
                            .point_position(*id)
                            .map(v2)
                            .ok_or(ProfileError::MissingPoint(*id))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let segment = ProfileSegment::BSpline {
                    control_points,
                    periodic: b.periodic,
                };
                if b.periodic {
                    wires.push(ProfileWire {
                        segments: vec![segment],
                    });
                } else {
                    edges.push(EdgeCurve {
                        ends: (
                            b.control_points[0],
                            *b.control_points.last().expect("len >= 2"),
                        ),
                        segment,
                    });
                }
            }
        }
    }

    if edges.is_empty() && wires.is_empty() {
        return Err(ProfileError::Empty);
    }

    // Endpoint graph: point id -> indices of edges touching it.
    let mut touching: HashMap<Uuid, Vec<usize>> = HashMap::new();
    for (idx, edge) in edges.iter().enumerate() {
        touching.entry(edge.ends.0).or_default().push(idx);
        touching.entry(edge.ends.1).or_default().push(idx);
    }
    for (point, list) in &touching {
        match list.len() {
            2 => {}
            1 => return Err(ProfileError::OpenAt(*point)),
            _ => return Err(ProfileError::BranchingAt(*point)),
        }
    }

    // Walk loops: every vertex has degree exactly 2, so each unvisited edge
    // starts a unique cycle.
    let mut used = vec![false; edges.len()];
    for start_idx in 0..edges.len() {
        if used[start_idx] {
            continue;
        }
        let mut segments = Vec::new();
        let start_point = edges[start_idx].ends.0;
        let mut current_point = start_point;
        let mut current_idx = start_idx;
        loop {
            used[current_idx] = true;
            let edge = &edges[current_idx];
            segments.push(edge.segment.clone());
            // Advance to the far end of this edge.
            current_point = if edge.ends.0 == current_point {
                edge.ends.1
            } else {
                edge.ends.0
            };
            if current_point == start_point {
                break;
            }
            // Exactly one other unused edge touches this point (degree 2).
            let next = touching[&current_point].iter().copied().find(|&i| !used[i]);
            match next {
                Some(i) => current_idx = i,
                // Degree checks above make this unreachable, but never trust
                // an invariant with a panic in production code.
                None => return Err(ProfileError::OpenAt(current_point)),
            }
        }
        wires.push(ProfileWire { segments });
    }

    Ok(wires)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Arc, Circle, Line, Point};

    fn pt(sketch: &mut Sketch, x: f32, y: f32) -> Uuid {
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
    }
    fn line(sketch: &mut Sketch, a: Uuid, b: Uuid) -> Uuid {
        sketch.add_geometry(GeometryElement::Line(Line::new(a, b)))
    }

    fn rectangle(sketch: &mut Sketch) -> [Uuid; 4] {
        let a = pt(sketch, 0.0, 0.0);
        let b = pt(sketch, 10.0, 0.0);
        let c = pt(sketch, 10.0, 5.0);
        let d = pt(sketch, 0.0, 5.0);
        line(sketch, a, b);
        line(sketch, b, c);
        line(sketch, c, d);
        line(sketch, d, a);
        [a, b, c, d]
    }

    #[test]
    fn rectangle_extracts_one_closed_wire() {
        let mut sketch = Sketch::new("t");
        rectangle(&mut sketch);
        let wires = extract_wires(&sketch).unwrap();
        assert_eq!(wires.len(), 1);
        assert_eq!(wires[0].segments.len(), 4);
    }

    #[test]
    fn circle_is_its_own_wire() {
        let mut sketch = Sketch::new("t");
        let c = pt(&mut sketch, 3.0, 3.0);
        sketch.add_geometry(GeometryElement::Circle(Circle::new(c, 2.0)));
        let wires = extract_wires(&sketch).unwrap();
        assert_eq!(wires.len(), 1);
        assert!(matches!(
            wires[0].segments[0],
            ProfileSegment::Circle { radius, .. } if (radius - 2.0).abs() < 1e-9
        ));
    }

    #[test]
    fn rectangle_with_hole_gives_two_wires() {
        let mut sketch = Sketch::new("t");
        rectangle(&mut sketch);
        let c = pt(&mut sketch, 5.0, 2.5);
        sketch.add_geometry(GeometryElement::Circle(Circle::new(c, 1.0)));
        let wires = extract_wires(&sketch).unwrap();
        assert_eq!(wires.len(), 2);
    }

    #[test]
    fn open_chain_is_rejected() {
        let mut sketch = Sketch::new("t");
        let a = pt(&mut sketch, 0.0, 0.0);
        let b = pt(&mut sketch, 10.0, 0.0);
        let c = pt(&mut sketch, 10.0, 5.0);
        line(&mut sketch, a, b);
        line(&mut sketch, b, c);
        assert!(matches!(
            extract_wires(&sketch),
            Err(ProfileError::OpenAt(_))
        ));
    }

    #[test]
    fn branching_is_rejected() {
        let mut sketch = Sketch::new("t");
        let [a, ..] = rectangle(&mut sketch);
        let e = pt(&mut sketch, -5.0, -5.0);
        let e2 = pt(&mut sketch, -5.0, 5.0);
        // Two extra edges through corner `a` (degree 4) forming a closed-ish
        // detour — every vertex except `a` has degree 2, so the failure is
        // unambiguously the branch at `a`.
        line(&mut sketch, a, e);
        line(&mut sketch, e, e2);
        line(&mut sketch, e2, a);
        assert!(matches!(
            extract_wires(&sketch),
            Err(ProfileError::BranchingAt(_))
        ));
    }

    #[test]
    fn dangling_edge_is_rejected() {
        let mut sketch = Sketch::new("t");
        let [a, ..] = rectangle(&mut sketch);
        let e = pt(&mut sketch, -5.0, -5.0);
        line(&mut sketch, a, e); // open spur off the rectangle
        assert!(matches!(
            extract_wires(&sketch),
            Err(ProfileError::OpenAt(_) | ProfileError::BranchingAt(_))
        ));
    }

    #[test]
    fn empty_sketch_is_rejected_but_points_are_ignored() {
        let mut sketch = Sketch::new("t");
        assert_eq!(extract_wires(&sketch), Err(ProfileError::Empty));
        pt(&mut sketch, 1.0, 1.0);
        assert_eq!(extract_wires(&sketch), Err(ProfileError::Empty));
    }

    #[test]
    fn construction_curves_are_ignored_by_profile() {
        let mut sketch = Sketch::new("t");
        let [a, _, c, _] = rectangle(&mut sketch);
        // A construction diagonal through two rectangle corners would make
        // the endpoint graph branch if it were considered part of the wire.
        let diagonal = line(&mut sketch, a, c);
        sketch.set_construction(diagonal, true);
        // A construction circle must not become its own wire either.
        let center = pt(&mut sketch, 5.0, 2.5);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 1.0)));
        sketch.set_construction(circle, true);

        let wires = extract_wires(&sketch).unwrap();
        assert_eq!(wires.len(), 1, "only the rectangle remains");
        assert_eq!(wires[0].segments.len(), 4);
    }

    #[test]
    fn all_construction_geometry_yields_empty_error() {
        let mut sketch = Sketch::new("t");
        let center = pt(&mut sketch, 0.0, 0.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 2.0)));
        sketch.set_construction(circle, true);
        assert_eq!(extract_wires(&sketch), Err(ProfileError::Empty));
    }

    #[test]
    fn arc_capped_slot_closes_and_mid_is_on_arc() {
        let mut sketch = Sketch::new("t");
        // A "slot": two horizontal lines capped by two semicircle arcs.
        let a = pt(&mut sketch, 0.0, 0.0);
        let b = pt(&mut sketch, 10.0, 0.0);
        let c = pt(&mut sketch, 10.0, 4.0);
        let d = pt(&mut sketch, 0.0, 4.0);
        let right_center = pt(&mut sketch, 10.0, 2.0);
        let left_center = pt(&mut sketch, 0.0, 2.0);
        line(&mut sketch, a, b);
        // CCW arc from b (10,0) to c (10,4) around (10,2) bulges right.
        sketch.add_geometry(GeometryElement::Arc(Arc::new(right_center, b, c, 2.0)));
        line(&mut sketch, c, d);
        sketch.add_geometry(GeometryElement::Arc(Arc::new(left_center, d, a, 2.0)));
        let wires = extract_wires(&sketch).unwrap();
        // The two arc centers are standalone-ish points but referenced by
        // arcs; the wire itself is the 4 curve segments.
        assert_eq!(wires.len(), 1);
        assert_eq!(wires[0].segments.len(), 4);
        let mid = wires[0]
            .segments
            .iter()
            .find_map(|s| match s {
                ProfileSegment::Arc { mid, .. } => Some(*mid),
                _ => None,
            })
            .unwrap();
        // Right cap bulge: mid should be at x = 12 (10 + r), y = 2.
        let on_right = (mid[0] - 12.0).abs() < 1e-4 && (mid[1] - 2.0).abs() < 1e-4;
        let on_left = (mid[0] + 2.0).abs() < 1e-4 && (mid[1] - 2.0).abs() < 1e-4;
        assert!(on_right || on_left, "arc mid off-curve: {mid:?}");
    }
}
