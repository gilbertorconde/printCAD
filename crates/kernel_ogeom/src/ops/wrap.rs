//! A sketch unrolled onto a cylinder or a cone, lengths kept.
//!
//! Both surfaces unroll flat without stretching. The sketch's plane is laid
//! against the surface where the middle of the sketch lands along its
//! normal, and rolled round it from there: on a cylinder of radius `r` a
//! step `s` across the axis turns by `s / r` and a step along the axis
//! stays one; on a cone the plane is the cone's flat sector, its apex the
//! cone's, turned onto the cone by the sector's angle over the cone's.
//!
//! Each curve is carried as a curve in the surface's chart composed with
//! the surface, so it lies on it exactly. On a cylinder the chart is the
//! sketch under an affine map, so the curve's exact B-spline is mapped
//! point for point; on a cone (and for a periodic spline) the chart curve
//! is fitted through the curve's mapped points.

use std::f64::consts::PI;

use ogeom::algo::{Containment, approximate_within, attach_pcurve, classify_on_face, make_edge};
use ogeom::geom::{
    BSpline2d, ConeSurface, Curve, Curve2d as _, Curve3d as _, CurveOnSurface, CylinderSurface,
    PlanarCurve, SurfaceGeometry, Transformable as _,
};
use ogeom::math::{
    Cone, Cylinder, Direction, Frame, Point, Point2, Transform2, Vector, Vector2, Weighted,
};
use ogeom::mesh::Deflection;
use ogeom::topo::{EdgeRepr, Location, Model, NodeData, Shape, SurfaceId};

use super::tol;
use crate::profile::BuiltWire;

/// How far the fitted chart curves may stand off the mapped points, mm.
const FIT_TOLERANCE: f64 = 1e-4;

/// The surface a sketch is unrolled onto.
#[derive(Clone, Copy)]
enum Kind {
    Cylinder {
        radius: f64,
    },
    Cone {
        /// `tan` of the half angle: the radius's growth per unit up the axis.
        slope: f64,
        reference_radius: f64,
    },
}

/// The unrolling of a sketch onto one cylinder or cone.
pub(super) struct Development {
    kind: Kind,
    /// The surface's frame, placed: `z` its axis, angles from `x` to `y`.
    frame: Frame,
    /// The carried curves' surface.
    surface: SurfaceGeometry,
    /// Where the sketch's middle lands: its angle and height in `frame`.
    angle: f64,
    height: f64,
    /// The sketch's middle, and its plane's directions laid across the axis
    /// (the way the angle grows) and along the surface up the axis.
    middle: Point,
    across: Vector,
    up: Vector,
}

impl Development {
    /// The unrolling onto the surface `face` lies on of `wires`, drawn on
    /// a plane whose normal is `along`.
    pub(super) fn of(
        model: &Model,
        face: &Shape,
        wires: &[&BuiltWire],
        along: Direction,
    ) -> Result<Self, String> {
        let (kind, frame) = surface_of(model, face)?;
        let (middle, size) = middle_of(model, wires)?;
        let n = along.vector();
        let local = |v: Vector| {
            Vector::new(
                frame.x().vector().dot(v),
                frame.y().vector().dot(v),
                frame.z().vector().dot(v),
            )
        };
        let world = |v: Vector| frame.x() * v.x + frame.y() * v.y + frame.z() * v.z;
        let p = frame.to_local(middle);
        let m = local(n);
        let landed = land(kind, p, m).ok_or_else(|| {
            format!(
                "the middle of the sketch does not land on the {} along its plane's normal",
                kind.name()
            )
        })?;
        let angle = landed.y.atan2(landed.x);
        let height = landed.z;
        let (sin, cos) = angle.sin_cos();
        let turning = world(Vector::new(-sin, cos, 0.0));
        let rising = match kind {
            Kind::Cylinder { .. } => world(Vector::new(0.0, 0.0, 1.0)),
            Kind::Cone { slope, .. } => {
                let g = Vector::new(slope * cos, slope * sin, 1.0);
                world(g * (1.0 / g.magnitude()))
            }
        };
        let flat = rising - n * rising.dot(n);
        if flat.magnitude() < 1e-6 {
            return Err(format!(
                "the sketch's plane stands square to the {}'s axis; wrapping needs \
                 a plane along it",
                kind.name()
            ));
        }
        let up = flat * (1.0 / flat.magnitude());
        let mut across = n.cross(up);
        if across.dot(turning) < 0.0 {
            across = -across;
        }
        Ok(Self {
            kind,
            frame,
            surface: carrier(kind, frame, height, size + 1.0)?,
            angle,
            height,
            middle,
            across,
            up,
        })
    }

    /// Whether `face` lies on the same surface as this unrolling.
    pub(super) fn holds(&self, model: &Model, face: &Shape) -> bool {
        let Ok((kind, frame)) = surface_of(model, face) else {
            return false;
        };
        let parallel = frame
            .z()
            .vector()
            .cross(self.frame.z().vector())
            .magnitude()
            < 1e-9;
        let off_axis = |p: Point| {
            let v = p - self.frame.origin();
            let a = self.frame.z().vector();
            (v - a * v.dot(a)).magnitude()
        };
        let same = |a: f64, b: f64| (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0);
        match (self.kind, kind) {
            (Kind::Cylinder { radius: r0 }, Kind::Cylinder { radius }) => {
                parallel && same(r0, radius) && off_axis(frame.origin()) < 1e-6
            }
            (Kind::Cone { .. }, Kind::Cone { .. }) => {
                parallel
                    && self.apex().distance(apex(kind, frame)) < 1e-6
                    && (self.opening() - opening(kind, frame)).magnitude() < 1e-9
            }
            _ => false,
        }
    }

    /// Where `face`'s own chart stands from this unrolling's: its surface,
    /// and the shift of angle and height from one chart to the other.
    /// `None` where the charts do not differ by a shift alone (the face
    /// placed, or its frame turned over).
    pub(super) fn chart_of(&self, model: &Model, face: &Shape) -> Option<(SurfaceId, Vector2)> {
        if !face.location().is_identity() || !self.holds(model, face) {
            return None;
        }
        let Some(NodeData::Face(data)) = model.node(face).map(|n| n.data()) else {
            return None;
        };
        let (_, frame) = surface_of(model, face).ok()?;
        let handed = |f: Frame| f.y().vector().dot(f.z().vector().cross(f.x().vector()));
        let (own, theirs) = (self.frame, frame);
        if own.z().vector().dot(theirs.z().vector()) <= 0.0 || handed(own) * handed(theirs) <= 0.0 {
            return None;
        }
        let x = own.x().vector();
        let turn = x.dot(theirs.y().vector()).atan2(x.dot(theirs.x().vector()));
        let rise = (own.origin() - theirs.origin()).dot(theirs.z().vector());
        Some((data.surface, Vector2::new(turn, rise)))
    }

    /// `wires`' edges unrolled onto the surface, each with points along it:
    /// an edge with corners comes back as one edge per smooth stretch. Each
    /// edge carries its chart curve on every surface of `charts`, so a face
    /// split along it reads its trace there exactly.
    pub(super) fn edges(
        &self,
        model: &mut Model,
        wires: &[&BuiltWire],
        charts: &[(SurfaceId, Vector2)],
    ) -> Result<Vec<(Shape, Vec<Point>)>, String> {
        let mut out = Vec::new();
        let stretches = wires
            .iter()
            .flat_map(|w| w.edges.iter())
            .map(|edge| curve_of(model, edge).map(|(curve, range)| smooth_stretches(curve, range)))
            .collect::<Result<Vec<_>, _>>()?;
        for (curve, range) in stretches.into_iter().flatten() {
            let chart = self.chart_curve(&curve, range)?;
            let domain = chart.domain();
            let carried = Curve::OnSurface(Box::new(CurveOnSurface::new(
                chart.clone(),
                self.surface.clone(),
            )));
            let at = |share: f64| {
                carried
                    .point_at((domain.1 - domain.0).mul_add(share, domain.0), tol())
                    .map_err(|e| format!("a wrapped curve does not evaluate: {e}"))
            };
            let coarse = (0..=16)
                .map(|k| at(f64::from(k) / 16.0))
                .collect::<Result<Vec<_>, _>>()?;
            let length: f64 = coarse.windows(2).map(|w| w[0].distance(w[1])).sum();
            // A point every quarter millimetre, so a narrow face the curve
            // crosses is found.
            let count = ((length / 0.25).ceil() as usize).clamp(16, 2000);
            let samples = (0..=count)
                .map(|k| at(k as f64 / count as f64))
                .collect::<Result<Vec<_>, _>>()?;
            let made = make_edge(model, carried, domain, tol())
                .map_err(|e| format!("making a wrapped curve's edge failed: {e}"))?;
            for &(surface, shift) in charts {
                let own = chart
                    .transformed(&Transform2::translation(shift), tol())
                    .map_err(|e| e.to_string())?;
                attach_pcurve(
                    model,
                    &made.shape,
                    own,
                    surface,
                    Location::identity(),
                    domain,
                )
                .map_err(|e| e.to_string())?;
            }
            // The space curve is the chart curve on the surface, so the two
            // agree at every parameter.
            if let Some(NodeData::Edge(data)) = model.node_mut(&made.shape).map(|n| n.data_mut()) {
                data.assert_same_parameter(true);
            }
            out.push((made.shape, samples));
        }
        Ok(out)
    }

    /// Whether any of `points` lies on `face`.
    pub(super) fn lands(model: &Model, face: &Shape, points: &[Point]) -> bool {
        points.iter().any(|&p| {
            classify_on_face(model, face, p, Deflection::default(), tol())
                .is_ok_and(|c| c != Containment::Out)
        })
    }

    /// `p`, a point on the surface, carried back onto the sketch's plane.
    pub(super) fn unwrap(&self, p: Point) -> Point {
        let local = self.frame.to_local(p);
        let turn = wrapped(local.y.atan2(local.x) - self.angle);
        let (s, w) = match self.kind {
            Kind::Cylinder { radius } => (turn * radius, local.z - self.height),
            Kind::Cone { .. } => {
                let (sector, sigma, cos) = self.sector();
                let apex = self.apex_height();
                let reach = sigma * (local.z - apex) / cos;
                let start = sigma * (self.height - apex) / cos;
                let phi = turn * sector;
                let (sin_phi, cos_phi) = phi.sin_cos();
                (reach * sin_phi, sigma * (reach * cos_phi - start))
            }
        };
        self.middle + self.across * s + self.up * w
    }

    /// Where `p`, a point on the sketch's plane, lands in the surface's
    /// chart: its angle and height.
    fn chart(&self, p: Point) -> Point2 {
        let d = p - self.middle;
        let (s, w) = (d.dot(self.across), d.dot(self.up));
        match self.kind {
            Kind::Cylinder { radius } => Point2::new(self.angle + s / radius, self.height + w),
            Kind::Cone { .. } => {
                let (sector, sigma, cos) = self.sector();
                let apex = self.apex_height();
                let start = sigma * (self.height - apex) / cos;
                let x = sigma.mul_add(w, start);
                let reach = x.hypot(s);
                let phi = s.atan2(x);
                Point2::new(
                    self.angle + phi / sector,
                    (sigma * reach).mul_add(cos, apex),
                )
            }
        }
    }

    /// The chart curve of `curve` over `range`, a curve on the sketch's
    /// plane.
    fn chart_curve(&self, curve: &Curve, range: (f64, f64)) -> Result<PlanarCurve, String> {
        if let Kind::Cylinder { .. } = self.kind
            && let Some(exact) = self.mapped_exactly(curve, range)
        {
            return Ok(exact);
        }
        self.fitted(curve, range)
    }

    /// `curve`'s exact B-spline with every control point mapped: the chart
    /// curve itself, where the chart is affine.
    fn mapped_exactly(&self, curve: &Curve, range: (f64, f64)) -> Option<PlanarCurve> {
        let spline = curve.to_bspline_over(range, tol()).ok()?;
        if spline.is_periodic() {
            return None;
        }
        let control = spline
            .control_points()
            .iter()
            .map(|w| Weighted::new(self.chart(w.point()), w.weight, tol()))
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        let mapped = BSpline2d::rational(spline.knots().clone(), control).ok()?;
        self.within_turn(&PlanarCurve::BSpline(mapped))
    }

    /// The chart curve fitted through `curve`'s mapped points, the angle
    /// scaled to a length round the surface where the sketch lands so the
    /// fit's tolerance reads in mm both ways.
    fn fitted(&self, curve: &Curve, range: (f64, f64)) -> Result<PlanarCurve, String> {
        let scale = self.radius_at(self.height).max(1e-3);
        let mut coarse = Vec::with_capacity(65);
        for k in 0..=64 {
            let t = (range.1 - range.0).mul_add(f64::from(k) / 64.0, range.0);
            coarse.push(curve.point_at(t, tol()).map_err(|e| e.to_string())?);
        }
        let length: f64 = coarse.windows(2).map(|w| w[0].distance(w[1])).sum();
        // A sample every quarter millimetre, within bounds.
        let count = ((length / 0.25).ceil() as usize).clamp(64, 4096);
        let mut points = Vec::with_capacity(count + 1);
        for k in 0..=count {
            let t = (range.1 - range.0).mul_add(k as f64 / count as f64, range.0);
            let p = curve.point_at(t, tol()).map_err(|e| e.to_string())?;
            let at = self.chart(p);
            points.push(Point::new(at.x * scale, at.y, 0.0));
        }
        let fitted = approximate_within(&points, FIT_TOLERANCE, tol())
            .map_err(|e| format!("fitting a wrapped curve failed: {e}"))?;
        let control = fitted
            .curve
            .control_points()
            .iter()
            .map(|w| {
                let p = w.point();
                Weighted::new(Point2::new(p.x / scale, p.y), w.weight, tol())
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let chart = BSpline2d::rational(fitted.curve.knots().clone(), control)
            .map_err(|e| format!("fitting a wrapped curve failed: {e}"))?;
        self.within_turn(&PlanarCurve::BSpline(chart))
            .ok_or_else(|| "the sketch wraps further than once round the surface".into())
    }

    /// `chart` if it stays within half a turn either side of where the
    /// sketch's middle lands, so it never overlaps itself.
    fn within_turn(&self, chart: &PlanarCurve) -> Option<PlanarCurve> {
        let (a, b) = chart.domain();
        let reaches = (0..=64).all(|k| {
            let t = (b - a).mul_add(f64::from(k) / 64.0, a);
            chart
                .point_at(t, tol())
                .is_ok_and(|p| (p.x - self.angle).abs() < PI)
        });
        reaches.then(|| chart.clone())
    }

    fn radius_at(&self, z: f64) -> f64 {
        match self.kind {
            Kind::Cylinder { radius } => radius,
            Kind::Cone {
                slope,
                reference_radius,
            } => slope.mul_add(z, reference_radius),
        }
    }

    /// The flat sector's angle per turn round the cone, which way up the
    /// axis leads away from the apex, and the half angle's cosine.
    fn sector(&self) -> (f64, f64, f64) {
        match self.kind {
            Kind::Cylinder { .. } => (1.0, 1.0, 1.0),
            Kind::Cone { slope, .. } => {
                let half = slope.atan();
                (half.sin().abs(), slope.signum(), half.cos())
            }
        }
    }

    fn apex_height(&self) -> f64 {
        match self.kind {
            Kind::Cylinder { .. } => 0.0,
            Kind::Cone {
                slope,
                reference_radius,
            } => -reference_radius / slope,
        }
    }

    fn apex(&self) -> Point {
        apex(self.kind, self.frame)
    }

    fn opening(&self) -> Vector {
        opening(self.kind, self.frame)
    }
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Cylinder { .. } => "cylinder",
            Kind::Cone { .. } => "cone",
        }
    }
}

fn apex(kind: Kind, frame: Frame) -> Point {
    match kind {
        Kind::Cylinder { .. } => frame.origin(),
        Kind::Cone {
            slope,
            reference_radius,
        } => frame.origin() - frame.z() * (reference_radius / slope),
    }
}

/// The way a cone widens: its axis, turned to point away from the apex.
fn opening(kind: Kind, frame: Frame) -> Vector {
    match kind {
        Kind::Cylinder { .. } => frame.z().vector(),
        Kind::Cone { slope, .. } => frame.z().vector() * slope.signum(),
    }
}

/// `angle` brought within half a turn of zero.
fn wrapped(angle: f64) -> f64 {
    let mut a = angle % (2.0 * PI);
    if a > PI {
        a -= 2.0 * PI;
    } else if a <= -PI {
        a += 2.0 * PI;
    }
    a
}

/// The cylinder or cone `face` lies on, and its frame, placed as the face
/// is.
fn surface_of(model: &Model, face: &Shape) -> Result<(Kind, Frame), String> {
    let Some(NodeData::Face(data)) = model.node(face).map(|n| n.data()) else {
        return Err("a picked face holds no face data".into());
    };
    let placement = face.transform(model.datums()).map_err(|e| e.to_string())?;
    let surface = model
        .geometry()
        .surface(data.surface)
        .ok_or("a picked face's surface is missing")?
        .transformed(&placement, tol())
        .map_err(|e| e.to_string())?;
    match surface {
        SurfaceGeometry::Cylinder(c) => {
            let c = c.cylinder();
            Ok((Kind::Cylinder { radius: c.radius() }, c.frame()))
        }
        SurfaceGeometry::Cone(c) => {
            let c = c.cone();
            Ok((
                Kind::Cone {
                    slope: c.half_angle().tan(),
                    reference_radius: c.reference_radius(),
                },
                c.frame(),
            ))
        }
        _ => Err("wrapping needs faces on a cylinder or a cone".into()),
    }
}

/// The surface the carried curves are drawn on: the face's, `reach` either
/// way along the axis from `height` (on a cone, no further than its apex),
/// past every point of the sketch; the surface's extent is what bounds of
/// the curves are read from.
fn carrier(kind: Kind, frame: Frame, height: f64, reach: f64) -> Result<SurfaceGeometry, String> {
    let (low, high) = (height - reach, height + reach);
    let made = match kind {
        Kind::Cylinder { radius } => Cylinder::new(frame, radius, tol())
            .and_then(|c| CylinderSurface::new(c, (low, high)))
            .map(SurfaceGeometry::Cylinder),
        Kind::Cone {
            slope,
            reference_radius,
        } => {
            let apex = -reference_radius / slope;
            let range = if slope > 0.0 {
                (low.max(apex), high)
            } else {
                (low, high.min(apex))
            };
            Cone::new(frame, reference_radius, slope.atan(), tol())
                .and_then(|c| ConeSurface::new(c, range))
                .map(SurfaceGeometry::Cone)
        }
    };
    made.map_err(|e| format!("the surface to wrap onto cannot be rebuilt: {e}"))
}

/// The middle of the box around `wires`' curves, and its diagonal.
fn middle_of(model: &Model, wires: &[&BuiltWire]) -> Result<(Point, f64), String> {
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for wire in wires {
        for edge in &wire.edges {
            let line = ogeom::mesh::polyline_of_edge(model, edge, Deflection::default(), tol())
                .map_err(|e| e.to_string())?;
            for p in line {
                for (k, c) in [p.x, p.y, p.z].into_iter().enumerate() {
                    low[k] = low[k].min(c);
                    high[k] = high[k].max(c);
                }
            }
        }
    }
    if !low[0].is_finite() {
        return Err("the sketch has no curves to wrap".into());
    }
    let middle = Point::new(
        (low[0] + high[0]) / 2.0,
        (low[1] + high[1]) / 2.0,
        (low[2] + high[2]) / 2.0,
    );
    Ok((
        middle,
        Point::new(low[0], low[1], low[2]).distance(Point::new(high[0], high[1], high[2])),
    ))
}

/// Where the line through `p` along `m` (both in the surface's frame) meets
/// the surface nearest `p`. On a cone only the nappe the frame's reference
/// circle is on counts.
fn land(kind: Kind, p: Point, m: Vector) -> Option<Point> {
    let (a, b, c) = match kind {
        Kind::Cylinder { radius } => (
            m.x.mul_add(m.x, m.y * m.y),
            2.0 * p.x.mul_add(m.x, p.y * m.y),
            p.x.mul_add(p.x, p.y * p.y) - radius * radius,
        ),
        Kind::Cone {
            slope,
            reference_radius,
        } => {
            let r = slope.mul_add(p.z, reference_radius);
            (
                m.x.mul_add(m.x, m.y * m.y) - slope * slope * m.z * m.z,
                2.0 * (p.x.mul_add(m.x, p.y * m.y) - slope * m.z * r),
                p.x.mul_add(p.x, p.y * p.y) - r * r,
            )
        }
    };
    let roots: Vec<f64> = if a.abs() < 1e-12 {
        if b.abs() < 1e-12 {
            Vec::new()
        } else {
            vec![-c / b]
        }
    } else {
        let disc = b.mul_add(b, -4.0 * a * c);
        if disc < 0.0 {
            Vec::new()
        } else {
            let root = disc.sqrt();
            vec![(-b - root) / (2.0 * a), (-b + root) / (2.0 * a)]
        }
    };
    roots
        .into_iter()
        .map(|t| (t, p + m * t))
        .filter(|(_, q)| match kind {
            Kind::Cylinder { .. } => true,
            Kind::Cone {
                slope,
                reference_radius,
            } => slope.mul_add(q.z, reference_radius) > 0.0,
        })
        .min_by(|x, y| x.0.abs().total_cmp(&y.0.abs()))
        .map(|(_, q)| q)
}

/// `curve` over `range` cut where its B-spline's derivative jumps: at a
/// corner, or where the pace along it changes at once. A carried curve's
/// offset, which a raised region's walls are built from, is fitted at the
/// curve's own parameter, and no smooth fit follows such a jump.
fn smooth_stretches(curve: Curve, range: (f64, f64)) -> Vec<(Curve, (f64, f64))> {
    let Ok(spline) = curve.to_bspline_over(range, tol()) else {
        return vec![(curve, range)];
    };
    let (lo, hi) = spline.knots().domain();
    let degree = spline.degree();
    let step = (hi - lo) * 1e-7;
    let jumps = |at: f64| match (
        spline.d1_at(at - step, tol()),
        spline.d1_at(at + step, tol()),
    ) {
        (Ok(a), Ok(b)) => (a - b).magnitude() > 1e-4 * a.magnitude().max(b.magnitude()),
        _ => false,
    };
    let mut cuts = vec![lo];
    cuts.extend(
        spline
            .knots()
            .distinct()
            .into_iter()
            .filter(|&(at, count)| at > lo + step && at < hi - step && count >= degree)
            .map(|(at, _)| at)
            .filter(|&at| jumps(at)),
    );
    cuts.push(hi);
    if cuts.len() == 2 {
        return vec![(curve, range)];
    }
    let spline = Curve::BSpline(spline);
    cuts.windows(2)
        .map(|w| (spline.clone(), (w[0], w[1])))
        .collect()
}

/// `edge`'s curve, placed, and the range of it the edge covers.
fn curve_of(model: &Model, edge: &Shape) -> Result<(Curve, (f64, f64)), String> {
    let Some(NodeData::Edge(data)) = model.node(edge).map(|n| n.data()) else {
        return Err("a sketch edge holds no edge data".into());
    };
    let Some(EdgeRepr::Curve3d { curve, range, .. }) = data.curve3d() else {
        return Err("a sketch edge has no curve".into());
    };
    let range = *range;
    let placement = edge.transform(model.datums()).map_err(|e| e.to_string())?;
    let curve = model
        .geometry()
        .curve(*curve)
        .ok_or("a sketch edge's curve is missing")?
        .transformed(&placement, tol())
        .map_err(|e| e.to_string())?;
    Ok((curve, range))
}
