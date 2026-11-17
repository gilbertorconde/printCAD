//! Splines edited by their nodes, as vector drawings edit paths: a cubic
//! spline whose every inner knot stands three times is a chain of Bézier
//! pieces, and the control point where two pieces meet lies on the curve,
//! a node, with the control points either side of it its handles. A node
//! is a corner, its handles free; smooth, its handles held in one line
//! through it (an angle of 180° at the node); or symmetric, its handles
//! mirrored through it. Those are constraints, so a solve keeps them.
//!
//! A closed path repeats its first control point as its last, one point,
//! so the curve closes on itself and a profile through it closes.

use uuid::Uuid;

use crate::sketch::{BSpline, ConstraintKind, GeometryElement, Sketch, Vec2D};

/// One node: its point on the curve and its handles, the one toward the
/// start and the one toward the end (an open path's ends have one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Node {
    pub point: Uuid,
    pub before: Option<Uuid>,
    pub after: Option<Uuid>,
}

impl Node {
    /// Its handles, toward the start first.
    pub fn handles(&self) -> impl Iterator<Item = Uuid> {
        self.before.into_iter().chain(self.after)
    }
}

/// A spline read as nodes, in order along it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nodes {
    pub spline: Uuid,
    pub nodes: Vec<Node>,
    pub closed: bool,
}

impl Nodes {
    pub fn node(&self, point: Uuid) -> Option<&Node> {
        self.nodes.iter().find(|n| n.point == point)
    }

    /// The node `handle` belongs to.
    pub fn owner(&self, handle: Uuid) -> Option<&Node> {
        self.nodes
            .iter()
            .find(|n| n.before == Some(handle) || n.after == Some(handle))
    }
}

/// How a node holds its handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Corner,
    Smooth,
    Symmetric,
}

impl NodeKind {
    pub const ALL: [NodeKind; 3] = [NodeKind::Corner, NodeKind::Smooth, NodeKind::Symmetric];

    pub fn label(self) -> &'static str {
        match self {
            NodeKind::Corner => "Corner",
            NodeKind::Smooth => "Smooth",
            NodeKind::Symmetric => "Symmetric",
        }
    }
}

/// `spline` read as nodes, when it is in node form: cubic, without
/// weights, open over clamped knots whose inner ones each stand three
/// times.
pub fn nodes_of(sketch: &Sketch, spline: Uuid) -> Option<Nodes> {
    let Some(GeometryElement::BSpline(b)) = sketch.get_geometry(spline) else {
        return None;
    };
    if !in_node_form(b) {
        return None;
    }
    let c = &b.control_points;
    let n = c.len();
    let closed = c[0] == c[n - 1];
    let last = if closed { n - 1 } else { n };
    let nodes = (0..last)
        .step_by(3)
        .map(|i| Node {
            point: c[i],
            before: match i {
                0 if closed => Some(c[n - 2]),
                0 => None,
                _ => Some(c[i - 1]),
            },
            after: (i + 1 < n).then(|| c[i + 1]),
        })
        .collect();
    Some(Nodes {
        spline,
        nodes,
        closed,
    })
}

fn in_node_form(b: &BSpline) -> bool {
    let n = b.control_points.len();
    if b.degree != 3 || !b.weights.is_empty() || b.periodic || n < 4 || !(n - 1).is_multiple_of(3) {
        return false;
    }
    if b.knots.is_empty() {
        return n == 4;
    }
    if b.knots.len() != n + 4 {
        return false;
    }
    let (d0, d1) = (b.knots[3], b.knots[n]);
    let mut inner: Vec<(f64, usize)> = Vec::new();
    for k in b.knots.iter().copied().filter(|k| *k > d0 && *k < d1) {
        match inner.last_mut() {
            Some((v, m)) if (*v - k).abs() < 1e-12 => *m += 1,
            _ => inner.push((k, 1)),
        }
    }
    inner.len() == (n - 4) / 3 && inner.iter().all(|(_, m)| *m == 3)
}

/// The clamped knots of a path of `pieces` Bézier pieces, over 0 to 1.
pub fn path_knots(pieces: usize) -> Vec<f64> {
    let pieces = pieces.max(1);
    let mut knots = vec![0.0; 4];
    for i in 1..pieces {
        knots.extend([i as f64 / pieces as f64; 3]);
    }
    knots.extend([1.0; 4]);
    knots
}

/// The constraint holding `node`'s handles as `kind` does, when it has
/// both.
fn kind_constraint(node: &Node, kind: NodeKind) -> Option<ConstraintKind> {
    let (Some(before), Some(after)) = (node.before, node.after) else {
        return None;
    };
    match kind {
        NodeKind::Corner => None,
        NodeKind::Smooth => Some(ConstraintKind::AngleThreePoints {
            point1: before,
            vertex: node.point,
            point2: after,
            angle_rad: std::f64::consts::PI,
        }),
        NodeKind::Symmetric => Some(ConstraintKind::SymmetricAboutPoint {
            point1: before,
            point2: after,
            center: node.point,
        }),
    }
}

/// Whether constraint `kind` is one of `node`'s own: its handles held
/// in line through it or mirrored through it.
fn holds(node: &Node, kind: &ConstraintKind) -> Option<NodeKind> {
    let (Some(before), Some(after)) = (node.before, node.after) else {
        return None;
    };
    let pair = |a: Uuid, b: Uuid| (a == before && b == after) || (a == after && b == before);
    match *kind {
        ConstraintKind::AngleThreePoints {
            point1,
            vertex,
            point2,
            angle_rad,
        } if vertex == node.point
            && pair(point1, point2)
            && (angle_rad.abs() - std::f64::consts::PI).abs() < 1e-9 =>
        {
            Some(NodeKind::Smooth)
        }
        ConstraintKind::SymmetricAboutPoint {
            point1,
            point2,
            center,
        } if center == node.point && pair(point1, point2) => Some(NodeKind::Symmetric),
        _ => None,
    }
}

/// How `node` holds its handles now.
pub fn kind_of(sketch: &Sketch, node: &Node) -> NodeKind {
    sketch
        .constraints
        .iter()
        .filter_map(|c| holds(node, &c.kind))
        .max_by_key(|k| *k as u8)
        .unwrap_or(NodeKind::Corner)
}

/// Make `node` hold its handles as `kind` says, in place of how it held
/// them; an end, with one handle, is always a corner. Answers whether
/// anything changed.
pub fn set_kind(sketch: &mut Sketch, node: &Node, kind: NodeKind) -> bool {
    if node.before.is_none() || node.after.is_none() || kind_of(sketch, node) == kind {
        return false;
    }
    sketch
        .constraints
        .retain(|c| holds(node, &c.kind).is_none());
    if let Some(constraint) = kind_constraint(node, kind) {
        sketch.add_constraint(constraint);
    }
    true
}

/// Hold smooth each node of `nodes` whose handles already lie in one line
/// through it, as a smooth spline's do when its knots are raised.
fn smooth_where_in_line(sketch: &mut Sketch, nodes: &Nodes) {
    for node in &nodes.nodes {
        let (Some(before), Some(after)) = (node.before, node.after) else {
            continue;
        };
        let at = |id| sketch.point_position(id).map(Vec2D::to_glam);
        let (Some(b), Some(p), Some(a)) = (at(before), at(node.point), at(after)) else {
            continue;
        };
        let (into, out) = (p - b, a - p);
        let scale = into.length() * out.length();
        if scale < 1e-18 {
            continue;
        }
        let in_line = into.perp_dot(out).abs() <= 1e-9 * scale && into.dot(out) > 0.0;
        if in_line && kind_of(sketch, node) == NodeKind::Corner {
            set_kind(sketch, node, NodeKind::Smooth);
        }
    }
}

/// Bring `spline` into node form for editing, the curve kept: answers
/// whether it had to be rewritten. Its nodes whose handles lie in line
/// come out smooth.
pub fn edit_nodes(sketch: &mut Sketch, spline: Uuid) -> Result<bool, String> {
    if nodes_of(sketch, spline).is_some() {
        return Ok(false);
    }
    if sketch.is_external(spline) {
        return Err("A projected spline follows its source; edit the source instead".into());
    }
    crate::spline_edit::to_node_form(sketch, spline)?;
    let nodes = nodes_of(sketch, spline).ok_or("This spline cannot be edited by its nodes")?;
    smooth_where_in_line(sketch, &nodes);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::Point;

    /// A sketch holding one spline over control points at `at`, as `b`
    /// sets it otherwise.
    fn spline(at: &[(f64, f64)], b: impl FnOnce(BSpline) -> BSpline) -> (Sketch, Uuid) {
        let mut sketch = Sketch::new("t");
        let ids: Vec<Uuid> = at
            .iter()
            .map(|&(x, y)| {
                sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
            })
            .collect();
        let id = sketch.add_geometry(GeometryElement::BSpline(b(BSpline::new(ids, false))));
        (sketch, id)
    }

    fn samples(sketch: &Sketch, id: Uuid) -> Vec<Vec2D> {
        let Some(GeometryElement::BSpline(b)) = sketch.get_geometry(id) else {
            panic!("a spline");
        };
        b.polyline(sketch, 64).expect("samples")
    }

    const WAVE: [(f64, f64); 6] = [
        (0.0, 0.0),
        (10.0, 10.0),
        (20.0, -5.0),
        (30.0, 12.0),
        (40.0, 0.0),
        (50.0, 6.0),
    ];

    /// A smooth cubic over even knots comes into node form as the same
    /// curve, each inner knot standing three times, every inner node held
    /// smooth since a smooth curve's handles lie in line through it.
    #[test]
    fn a_smooth_spline_becomes_the_same_curve_in_smooth_nodes() {
        let (mut sketch, id) = spline(&WAVE, |b| b);
        let before = samples(&sketch, id);
        assert!(nodes_of(&sketch, id).is_none(), "not in node form yet");
        assert_eq!(edit_nodes(&mut sketch, id), Ok(true));
        let after = samples(&sketch, id);
        let worst = before
            .iter()
            .zip(&after)
            .map(|(a, b)| (*a - *b).to_glam().length())
            .fold(0.0, f64::max);
        assert!(worst < 1e-9, "the curve moved by {worst}");
        let nodes = nodes_of(&sketch, id).expect("node form");
        // Six control points over even cubic knots: three pieces.
        assert_eq!(nodes.nodes.len(), 4);
        assert!(!nodes.closed);
        let kinds: Vec<NodeKind> = nodes.nodes.iter().map(|n| kind_of(&sketch, n)).collect();
        use NodeKind::*;
        assert_eq!(
            kinds,
            [Corner, Smooth, Smooth, Corner],
            "ends have one handle"
        );
        assert_eq!(edit_nodes(&mut sketch, id), Ok(false), "already nodes");
    }

    /// A node's kind is one constraint: smooth holds its handles in line,
    /// symmetric mirrors them, a corner holds nothing; changing it swaps
    /// the one for the other.
    #[test]
    fn a_node_holds_its_handles_as_its_kind_says() {
        let (mut sketch, id) = spline(&WAVE, |b| b);
        edit_nodes(&mut sketch, id).unwrap();
        let node = nodes_of(&sketch, id).unwrap().nodes[1];
        let held = |sketch: &Sketch| {
            sketch
                .constraints
                .iter()
                .filter(|c| holds(&node, &c.kind).is_some())
                .count()
        };
        for kind in [NodeKind::Symmetric, NodeKind::Corner, NodeKind::Smooth] {
            assert!(set_kind(&mut sketch, &node, kind));
            assert_eq!(kind_of(&sketch, &node), kind);
            assert_eq!(held(&sketch), usize::from(kind != NodeKind::Corner));
        }
        assert!(!set_kind(&mut sketch, &node, NodeKind::Smooth), "no change");
        let end = nodes_of(&sketch, id).unwrap().nodes[0];
        assert!(
            !set_kind(&mut sketch, &end, NodeKind::Smooth),
            "an end has one handle"
        );
    }

    /// A closed spline comes into node form laid open at its start, its
    /// first and last control point one point, so the curve still closes.
    #[test]
    fn a_closed_spline_keeps_closing_in_node_form() {
        let square = [(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)];
        let (mut sketch, id) = spline(&square, |b| BSpline {
            periodic: true,
            ..b
        });
        let before = samples(&sketch, id);
        edit_nodes(&mut sketch, id).unwrap();
        let nodes = nodes_of(&sketch, id).expect("node form");
        assert!(nodes.closed);
        assert!(
            nodes.nodes[0].before.is_some(),
            "the start node has both handles"
        );
        // The same loop, wherever along it the samples start.
        for p in samples(&sketch, id) {
            let near = before
                .iter()
                .map(|q| (p - *q).to_glam().length())
                .fold(f64::INFINITY, f64::min);
            assert!(near < 0.5, "{p:?} is {near} off the loop");
        }
    }

    #[test]
    fn a_spline_with_weights_is_refused() {
        let (mut sketch, id) = spline(&WAVE, |b| BSpline {
            weights: vec![1.0, 2.0, 1.0, 1.0, 1.0, 1.0],
            ..b
        });
        assert!(edit_nodes(&mut sketch, id).is_err());
    }

    #[test]
    fn a_path_s_knots_stand_three_times_inside() {
        assert_eq!(path_knots(1), [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]);
        let k = path_knots(2);
        assert_eq!(k.len(), 7 + 4);
        assert_eq!(&k[4..7], &[0.5; 3]);
    }
}
