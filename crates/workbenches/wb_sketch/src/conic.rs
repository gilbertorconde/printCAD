//! Parabola and hyperbola arcs: points along them, the parameter of a point
//! on them, how far a point is from them, and the exact rational quadratic
//! the kernel draws them with. Worked in double precision.

use crate::sketch::{Conic, ConicKind, Sketch, Vec2D};

/// A conic's frame and shape in double precision.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    pub kind: ConicKind,
    /// The parabola's vertex, the hyperbola's centre.
    pub origin: [f64; 2],
    /// Unit vector along the axis, the way the curve opens.
    pub u: [f64; 2],
    /// The focal distance of a parabola, the semi-major axis of a hyperbola.
    pub a: f64,
    /// The hyperbola's semi-minor axis.
    pub b: f64,
}

fn add(p: [f64; 2], q: [f64; 2]) -> [f64; 2] {
    [p[0] + q[0], p[1] + q[1]]
}

fn scale(p: [f64; 2], s: f64) -> [f64; 2] {
    [p[0] * s, p[1] * s]
}

impl Shape {
    /// The shape of `kind` at `origin` with `axis` (its length the focal
    /// distance or semi-major axis) and `minor`. `None` when degenerate.
    pub fn new(kind: ConicKind, origin: Vec2D, axis: Vec2D, minor: f64) -> Option<Self> {
        let (ax, ay) = (axis.x, axis.y);
        let a = ax.hypot(ay);
        let b = minor;
        if a < 1e-9 || (kind == ConicKind::Hyperbola && b < 1e-9) {
            return None;
        }
        Some(Self {
            kind,
            origin: [origin.x, origin.y],
            u: [ax / a, ay / a],
            a,
            b,
        })
    }

    /// The shape of `conic` where its centre is in `sketch`.
    pub fn of(conic: &Conic, sketch: &Sketch) -> Option<Self> {
        Self::new(
            conic.kind,
            sketch.point_position(conic.center)?,
            conic.axis,
            conic.minor,
        )
    }

    /// The left-hand perpendicular of the axis.
    fn v(&self) -> [f64; 2] {
        [-self.u[1], self.u[0]]
    }

    /// A point in the curve's own frame, `(x, y)`, in the sketch.
    fn at_local(&self, x: f64, y: f64) -> [f64; 2] {
        add(self.origin, add(scale(self.u, x), scale(self.v(), y)))
    }

    /// A sketch point in the curve's own frame.
    fn local_of(&self, p: [f64; 2]) -> (f64, f64) {
        let d = [p[0] - self.origin[0], p[1] - self.origin[1]];
        let v = self.v();
        (
            d[0] * self.u[0] + d[1] * self.u[1],
            d[0] * v[0] + d[1] * v[1],
        )
    }

    /// The point at parameter `t`.
    pub fn point(&self, t: f64) -> [f64; 2] {
        match self.kind {
            ConicKind::Parabola => self.at_local(t * t / (4.0 * self.a), t),
            ConicKind::Hyperbola => self.at_local(self.a * t.cosh(), self.b * t.sinh()),
        }
    }

    /// The parameter of the point on the curve level with `p` across the
    /// axis: exact for a point on it.
    pub fn param(&self, p: [f64; 2]) -> f64 {
        let (_, y) = self.local_of(p);
        match self.kind {
            ConicKind::Parabola => y,
            ConicKind::Hyperbola => (y / self.b).asinh(),
        }
    }

    /// The curve's equation at `p` divided by its gradient's length: near
    /// the curve, the signed distance to it.
    pub fn distance(&self, p: [f64; 2]) -> f64 {
        let (x, y) = self.local_of(p);
        match self.kind {
            ConicKind::Parabola => {
                let f = x - y * y / (4.0 * self.a);
                f / (1.0 + (y / (2.0 * self.a)).powi(2)).sqrt()
            }
            ConicKind::Hyperbola => {
                let (a2, b2) = (self.a * self.a, self.b * self.b);
                let f = x * x / a2 - y * y / b2 - 1.0;
                let g = ((2.0 * x / a2).powi(2) + (2.0 * y / b2).powi(2)).sqrt();
                f / g.max(1e-12)
            }
        }
    }

    /// `segments + 1` points from parameter `t0` to `t1`.
    pub fn sample(&self, t0: f64, t1: f64, segments: usize) -> Vec<Vec2D> {
        let n = segments.max(1);
        (0..=n)
            .map(|i| {
                let [x, y] = self.point(t0 + (t1 - t0) * i as f64 / n as f64);
                Vec2D::new(x, y)
            })
            .collect()
    }

    /// The arc from `t0` to `t1` as a rational quadratic: its three control
    /// points and the middle one's weight, which is exactly the curve.
    pub fn quadratic(&self, t0: f64, t1: f64) -> ([[f64; 2]; 3], f64) {
        match self.kind {
            ConicKind::Parabola => {
                // The tangents at the ends meet at the middle control point.
                let tangent = self.at_local(t0 / (2.0 * self.a), 1.0);
                let dir = [tangent[0] - self.origin[0], tangent[1] - self.origin[1]];
                let p0 = self.point(t0);
                let p1 = add(p0, scale(dir, (t1 - t0) / 2.0));
                ([p0, p1, self.point(t1)], 1.0)
            }
            ConicKind::Hyperbola => {
                let (m, h) = ((t0 + t1) / 2.0, (t1 - t0) / 2.0);
                let w = h.cosh();
                let p1 = self.at_local(self.a * m.cosh() / w, self.b * m.sinh() / w);
                ([self.point(t0), p1, self.point(t1)], w)
            }
        }
    }
}

/// The hyperbola centred at `center` with the vertex of its branch at
/// `vertex` that passes through `on`: its semi-minor axis. `None` when `on`
/// lies no further out along the axis than the vertex.
pub fn hyperbola_minor(center: Vec2D, vertex: Vec2D, on: Vec2D) -> Option<f64> {
    let axis = (vertex - center).to_glam();
    let a = axis.length();
    if a < 1e-6 {
        return None;
    }
    let u = axis / a;
    let d = (on - center).to_glam();
    let (x, y) = (d.dot(u), d.dot(u.perp()));
    let s = (x / a).powi(2) - 1.0;
    if x <= a || s <= 1e-9 || y.abs() < 1e-6 {
        return None;
    }
    Some(y.abs() / s.sqrt())
}

impl Conic {
    /// The arc's parameter span, start to end.
    pub fn params(&self, sketch: &Sketch) -> Option<(Shape, f64, f64)> {
        let shape = Shape::of(self, sketch)?;
        let at = |id| sketch.point_position(id).map(|p| [p.x, p.y]);
        Some((
            shape,
            shape.param(at(self.start)?),
            shape.param(at(self.end)?),
        ))
    }

    /// The arc sampled at `segments` intervals, start to end.
    pub fn points(&self, sketch: &Sketch, segments: usize) -> Option<Vec<Vec2D>> {
        let (shape, t0, t1) = self.params(sketch)?;
        Some(shape.sample(t0, t1, segments))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(p: [f64; 2], q: [f64; 2]) -> bool {
        (p[0] - q[0]).hypot(p[1] - q[1]) < 1e-9
    }

    /// The rational quadratic at `s` in [0, 1].
    fn bezier(cp: [[f64; 2]; 3], w: f64, s: f64) -> [f64; 2] {
        let (b0, b1, b2) = ((1.0 - s).powi(2), 2.0 * s * (1.0 - s) * w, s * s);
        let d = b0 + b1 + b2;
        [
            (b0 * cp[0][0] + b1 * cp[1][0] + b2 * cp[2][0]) / d,
            (b0 * cp[0][1] + b1 * cp[1][1] + b2 * cp[2][1]) / d,
        ]
    }

    fn shapes() -> [Shape; 2] {
        let axis = Vec2D::new(1.2, 1.6);
        [
            Shape::new(ConicKind::Parabola, Vec2D::new(1.0, -2.0), axis, 0.0).unwrap(),
            Shape::new(ConicKind::Hyperbola, Vec2D::new(1.0, -2.0), axis, 1.5).unwrap(),
        ]
    }

    #[test]
    fn a_point_on_the_curve_gives_back_its_parameter_and_lies_on_it() {
        for shape in shapes() {
            for t in [-1.5, -0.2, 0.0, 0.7, 2.0] {
                let p = shape.point(t);
                assert!((shape.param(p) - t).abs() < 1e-9, "{:?} at {t}", shape.kind);
                assert!(shape.distance(p).abs() < 1e-9);
            }
            // Off the curve, the distance is about how far off.
            let p = shape.point(0.4);
            let normal_step = [p[0] + 1e-3 * shape.u[0], p[1] + 1e-3 * shape.u[1]];
            assert!(shape.distance(normal_step).abs() > 1e-4);
        }
    }

    #[test]
    fn the_rational_quadratic_is_the_arc_exactly() {
        for shape in shapes() {
            for (t0, t1) in [(-1.0, 1.5), (0.3, -2.0)] {
                let (cp, w) = shape.quadratic(t0, t1);
                assert!(close(cp[0], shape.point(t0)) && close(cp[2], shape.point(t1)));
                for i in 0..=10 {
                    let q = bezier(cp, w, i as f64 / 10.0);
                    assert!(
                        shape.distance(q).abs() < 1e-9,
                        "{:?}: {q:?} off by {}",
                        shape.kind,
                        shape.distance(q)
                    );
                }
            }
        }
    }

    #[test]
    fn a_hyperbola_through_a_point_beyond_its_vertex() {
        let b = hyperbola_minor(
            Vec2D::new(0.0, 0.0),
            Vec2D::new(3.0, 0.0),
            Vec2D::new(5.0, 2.0),
        )
        .unwrap();
        let shape = Shape::new(
            ConicKind::Hyperbola,
            Vec2D::new(0.0, 0.0),
            Vec2D::new(3.0, 0.0),
            b,
        )
        .unwrap();
        assert!(shape.distance([5.0, 2.0]).abs() < 1e-5);
        assert!(
            hyperbola_minor(
                Vec2D::new(0.0, 0.0),
                Vec2D::new(3.0, 0.0),
                Vec2D::new(2.0, 2.0)
            )
            .is_none(),
            "inside the vertex no branch passes"
        );
    }
}
