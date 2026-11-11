//! A sketch wrapped onto a curved face: unrolled onto a cylinder or a
//! cone, lengths kept, or laid as a flat sheet onto any other surface.
//!
//! A cylinder and a cone unroll flat without stretching. The sketch's
//! plane is laid against the surface where the middle of the sketch lands
//! along its normal, and rolled round it from there: on a cylinder of
//! radius `r` a step `s` across the axis turns by `s / r` and a step along
//! the axis stays one; on a cone the plane is the cone's flat sector, its
//! apex the cone's, turned onto the cone by the sector's angle over the
//! cone's.
//!
//! Every other surface (a sphere, a torus, a free-form face) takes the
//! sketch through the kernel's `SurfaceSheet`, laid by geodesic polar
//! coordinates: its origin where the sketch's middle lands, its `+x` the
//! sketch's x direction there. Lengths along every line through the middle
//! are kept, and lengths across those lines stretch as the surface's
//! curvature makes them. A point on the surface is carried back to the
//! sketch by Gauss-Newton steps on the sheet's chart.
//!
//! Each curve is carried as a curve in the surface's chart composed with
//! the surface, so it lies on it exactly. On a cylinder the chart is the
//! sketch under an affine map, so the curve's exact B-spline is mapped
//! point for point; on a cone (and for a periodic spline) the chart curve
//! is fitted through the curve's mapped points, and on a sheet the kernel
//! fits it.

use std::f64::consts::PI;

use ogeom::algo::{
    Containment, SheetMethod, SheetPlacement, SurfaceSheet, approximate_within, attach_pcurve,
    classify_on_face, make_edge,
};
use ogeom::geom::{
    BSpline2d, ConeSurface, Curve, Curve2d as _, Curve3d as _, CurveOnSurface, CylinderSurface,
    PlanarCurve, Surface as _, SurfaceGeometry, Transformable as _, project_on_surface,
};
use ogeom::math::{
    Cone, Cylinder, Direction, Frame, Point, Point2, Transform2, Vector, Vector2, Weighted,
};
use ogeom::mesh::{Deflection, triangulate_face};
use ogeom::topo::{EdgeRepr, Location, Model, NodeData, Orientation, Shape, SurfaceId};

use super::tol;
use crate::convert::{array, point};
use crate::profile::BuiltWire;

/// How far the fitted chart curves may stand off the mapped points, mm.
const FIT_TOLERANCE: f64 = 1e-4;

/// How a sheet is carried away from its origin on a surface that does
/// not unroll: by geodesic polar coordinates, whose radius on the sheet is
/// the distance along the surface from the origin, which [`Laid::unwrap`]
/// reads.
const SHEET_METHOD: SheetMethod = SheetMethod::Geodesic;

/// The share of the sketch's own reach a sheet reaches past it.
const SHEET_MARGIN: f64 = 0.02;

/// The most Gauss-Newton steps a point carried back from a sheet takes,
/// and how near the surface point it must land, mm.
const UNWRAP_STEPS: usize = 40;
const UNWRAP_ACCURACY: f64 = 1e-6;

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
pub(super) struct Unrolling {
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

impl Unrolling {
    /// The unrolling onto the cylinder or cone `kind` in `frame` of
    /// `wires`, drawn on a plane whose normal is `along`.
    fn of(
        model: &Model,
        kind: Kind,
        frame: Frame,
        wires: &[&BuiltWire],
        along: Direction,
    ) -> Result<Self, String> {
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
    fn holds(&self, model: &Model, face: &Shape) -> bool {
        let Ok(Some((kind, frame))) = surface_of(model, face) else {
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
    fn chart_of(&self, model: &Model, face: &Shape) -> Option<(SurfaceId, Vector2)> {
        if !face.location().is_identity() || !self.holds(model, face) {
            return None;
        }
        let Some(NodeData::Face(data)) = model.node(face).map(|n| n.data()) else {
            return None;
        };
        let (_, frame) = surface_of(model, face).ok()??;
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
    fn edges(
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

    /// `p`, a point on the surface, carried back onto the sketch's plane.
    fn unwrap(&self, p: Point) -> Point {
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

/// How a sketch is carried onto the picked faces: unrolled onto a
/// cylinder or cone, or laid as a flat sheet onto any other curved
/// surface.
pub(super) enum Development {
    Unrolled(Box<Unrolling>),
    Laid(Box<Laid>),
}

impl Development {
    /// The carrying of `wires`, drawn on a plane whose normal is `along`
    /// and whose x direction is `x_axis`, onto the surface the first of
    /// `faces` lies on.
    pub(super) fn of(
        model: &Model,
        faces: &[Shape],
        wires: &[&BuiltWire],
        along: Direction,
        x_axis: Vector,
    ) -> Result<Self, String> {
        let first = faces
            .first()
            .ok_or("pick the faces to wrap the sketch onto")?;
        match surface_of(model, first)? {
            Some((kind, frame)) => {
                Unrolling::of(model, kind, frame, wires, along).map(|u| Self::Unrolled(Box::new(u)))
            }
            None => Laid::of(model, faces, wires, along, x_axis).map(|l| Self::Laid(Box::new(l))),
        }
    }

    /// Whether `face` lies on the same surface as this carrying.
    pub(super) fn holds(&self, model: &Model, face: &Shape) -> bool {
        match self {
            Self::Unrolled(u) => u.holds(model, face),
            Self::Laid(l) => l.holds(model, face),
        }
    }

    /// Where `face`'s own chart stands from an unrolling's (see
    /// [`Unrolling::chart_of`]); `None` for a sheet, which carries its
    /// chart curves on the surface it was laid on.
    pub(super) fn chart_of(&self, model: &Model, face: &Shape) -> Option<(SurfaceId, Vector2)> {
        match self {
            Self::Unrolled(u) => u.chart_of(model, face),
            Self::Laid(_) => None,
        }
    }

    /// `wires`' edges carried onto the surface, each with points along it.
    pub(super) fn edges(
        &self,
        model: &mut Model,
        wires: &[&BuiltWire],
        charts: &[(SurfaceId, Vector2)],
    ) -> Result<Vec<(Shape, Vec<Point>)>, String> {
        match self {
            Self::Unrolled(u) => u.edges(model, wires, charts),
            Self::Laid(l) => l.edges(model, wires),
        }
    }

    /// Whether any of `points` lies on `face`.
    pub(super) fn lands(model: &Model, face: &Shape, points: &[Point]) -> bool {
        points.iter().any(|&p| {
            classify_on_face(model, face, p, Deflection::default(), tol())
                .is_ok_and(|c| c != Containment::Out)
        })
    }

    /// `p`, a point on the surface, carried back onto the sketch's plane;
    /// `None` where it lies past the part of the surface the sketch was
    /// carried over, so no region of the sketch covers it.
    pub(super) fn unwrap(&self, p: Point) -> Result<Option<Point>, String> {
        match self {
            Self::Unrolled(u) => Ok(Some(u.unwrap(p))),
            Self::Laid(l) => l.unwrap(p),
        }
    }
}

/// A sketch laid as a flat sheet onto a surface that does not unroll.
pub(super) struct Laid {
    sheet: SurfaceSheet,
    /// The face the sheet is laid on.
    face: Shape,
    /// Where the sheet's origin lands, and the sheet's `+x` and `+y`
    /// there, tangent to the surface.
    at: Point,
    x: Vector,
    y: Vector,
    reach: f64,
    /// The sketch's middle, and its plane's directions the sheet's `+x` and
    /// `+y` stand for.
    middle: Point,
    across: Vector,
    up: Vector,
}

impl Laid {
    fn of(
        model: &Model,
        faces: &[Shape],
        wires: &[&BuiltWire],
        along: Direction,
        x_axis: Vector,
    ) -> Result<Self, String> {
        let (middle, size) = middle_of(model, wires)?;
        let n = along.vector();
        let (face, at, normal) = landing(model, faces, middle, along)?;
        let across = x_axis - n * x_axis.dot(n);
        if across.magnitude() < 1e-9 {
            return Err("the sketch's x direction stands along its plane's normal".into());
        }
        let across = across * (1.0 / across.magnitude());
        let x = across - normal * across.dot(normal);
        if x.magnitude() < 1e-6 {
            return Err(
                "the sketch's x direction stands square to the face where its middle lands".into(),
            );
        }
        let x = x * (1.0 / x.magnitude());
        let y = normal.cross(x);
        let mut up = n.cross(across);
        if up.dot(y) < 0.0 {
            up = -up;
        }
        // Every point of the sketch lies within half the box's diagonal of
        // its middle.
        let reach = (size / 2.0).max(tol().confusion()) * (1.0 + SHEET_MARGIN);
        let placement = SheetPlacement {
            at,
            x_direction: Direction::new(across, tol()).map_err(|e| e.to_string())?,
            method: SHEET_METHOD,
        };
        let sheet = SurfaceSheet::new(model, &face, &placement, reach, tol())
            .map_err(|e| format!("the sketch cannot be laid onto the face: {e}"))?;
        Ok(Self {
            sheet,
            face,
            at,
            x,
            y,
            reach,
            middle,
            across,
            up,
        })
    }

    /// Whether `face` lies on the surface the sheet is laid on: the same
    /// surface, or points of it on the sheet's.
    fn holds(&self, model: &Model, face: &Shape) -> bool {
        let surface_id = |f: &Shape| match model.node(f).map(|n| n.data()) {
            Some(NodeData::Face(data)) => Some(data.surface),
            _ => None,
        };
        if face.node() == self.face.node()
            || (face.location() == self.face.location()
                && surface_id(face).is_some()
                && surface_id(face) == surface_id(&self.face))
        {
            return true;
        }
        let Ok(drawn) = triangulate_face(model, face, Deflection::default(), tol()) else {
            return false;
        };
        let step = (drawn.positions.len() / 24).max(1);
        drawn.positions.iter().step_by(step).all(|&p| {
            project_on_surface(self.sheet.surface(), p, 24, tol())
                .is_ok_and(|found| found.distance < 1e-6)
        })
    }

    /// `wires`' edges laid onto the surface, each with points along it, cut
    /// as [`Unrolling::edges`] cuts them and where a curve runs through a
    /// pole of the surface's chart.
    fn edges(
        &self,
        model: &mut Model,
        wires: &[&BuiltWire],
    ) -> Result<Vec<(Shape, Vec<Point>)>, String> {
        let stretches = wires
            .iter()
            .flat_map(|w| w.edges.iter())
            .map(|edge| curve_of(model, edge).map(|(curve, range)| smooth_stretches(curve, range)))
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = Vec::new();
        for (curve, range) in stretches.into_iter().flatten() {
            let (planar, range) = self.on_sheet(&curve, range)?;
            let made = self
                .sheet
                .chart_edges(model, &planar, range, FIT_TOLERANCE)
                .map_err(|e| format!("laying a curve of the sketch onto the face failed: {e}"))?;
            for edge in made {
                let samples = samples_of(model, &edge)?;
                out.push((edge, samples));
            }
        }
        Ok(out)
    }

    /// `curve` over `range`, a curve on the sketch's plane, as a curve on
    /// the sheet, with the range of its parameter that covers it: its exact
    /// B-spline with every control point mapped, the map from the plane to
    /// the sheet being a rigid motion, or fitted through its mapped points
    /// where that spline is periodic.
    fn on_sheet(
        &self,
        curve: &Curve,
        range: (f64, f64),
    ) -> Result<(PlanarCurve, (f64, f64)), String> {
        let flat = |p: Point| {
            let d = p - self.middle;
            Point2::new(d.dot(self.across), d.dot(self.up))
        };
        if let Ok(spline) = curve.to_bspline_over(range, tol())
            && !spline.is_periodic()
        {
            let control = spline
                .control_points()
                .iter()
                .map(|w| Weighted::new(flat(w.point()), w.weight, tol()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if let Ok(mapped) = BSpline2d::rational(spline.knots().clone(), control) {
                let domain = spline.knots().domain();
                return Ok((PlanarCurve::BSpline(mapped), domain));
            }
        }
        let count = 512;
        let mut points = Vec::with_capacity(count + 1);
        for k in 0..=count {
            let t = (range.1 - range.0).mul_add(k as f64 / count as f64, range.0);
            let p = flat(curve.point_at(t, tol()).map_err(|e| e.to_string())?);
            points.push(Point::new(p.x, p.y, 0.0));
        }
        let fitted = approximate_within(&points, FIT_TOLERANCE / 4.0, tol())
            .map_err(|e| format!("fitting a curve of the sketch failed: {e}"))?;
        let control = fitted
            .curve
            .control_points()
            .iter()
            .map(|w| {
                let p = w.point();
                Weighted::new(Point2::new(p.x, p.y), w.weight, tol())
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let domain = fitted.curve.knots().domain();
        let chart = BSpline2d::rational(fitted.curve.knots().clone(), control)
            .map_err(|e| format!("fitting a curve of the sketch failed: {e}"))?;
        Ok((PlanarCurve::BSpline(chart), domain))
    }

    /// Where the sheet point `q` lands in space.
    fn landed(&self, q: Point2) -> Result<Point, String> {
        let uv = self.sheet.chart_point(q).map_err(|e| e.to_string())?;
        self.sheet
            .surface()
            .point_at(uv.x, uv.y, tol())
            .map_err(|e| e.to_string())
    }

    /// `p`, a point near the surface, carried back onto the sketch's plane:
    /// the sheet point landing where `p` meets the surface, found by
    /// Gauss-Newton steps on the sheet's chart from where `p` stands in the
    /// tangent plane at the sheet's origin. `None` where `p` lies past the
    /// sheet's reach: the sheet's radius is the distance along the surface
    /// from its origin, never shorter than the straight one.
    fn unwrap(&self, p: Point) -> Result<Option<Point>, String> {
        let p = project_on_surface(self.sheet.surface(), p, 24, tol())
            .map_err(|e| e.to_string())?
            .point;
        if p.distance(self.at) > self.reach {
            return Ok(None);
        }
        let lost = || "a point of the face cannot be carried back onto the sketch's plane";
        let edge = self.reach * (1.0 - 1e-9);
        let clamp = |q: Point2| {
            let r = q.x.hypot(q.y);
            if r > edge {
                (Point2::new(q.x * edge / r, q.y * edge / r), true)
            } else {
                (q, false)
            }
        };
        let d = p - self.at;
        let (mut q, mut at_edge) = clamp(Point2::new(d.dot(self.x), d.dot(self.y)));
        let h = 1e-6 * self.reach;
        for _ in 0..UNWRAP_STEPS {
            let miss = p - self.landed(q)?;
            if miss.magnitude() <= UNWRAP_ACCURACY {
                return Ok(Some(self.middle + self.across * q.x + self.up * q.y));
            }
            // Central differences, stepped inward where `q` stands at the
            // reach.
            let column = |dq: Vector2| -> Result<Vector, String> {
                let out = q + dq;
                let (ahead, back) = if out.x.hypot(out.y) > edge {
                    (q, q - dq)
                } else {
                    (out, q - dq)
                };
                Ok((self.landed(ahead)? - self.landed(back)?) * (1.0 / (ahead - back).magnitude()))
            };
            let ju = column(Vector2::new(h, 0.0))?;
            let jv = column(Vector2::new(0.0, h))?;
            let (a, b, c) = (ju.dot(ju), ju.dot(jv), jv.dot(jv));
            let det = a.mul_add(c, -b * b);
            if det <= f64::EPSILON * a * c {
                return Err(lost().into());
            }
            let (ru, rv) = (ju.dot(miss), jv.dot(miss));
            let mut step = Vector2::new(c.mul_add(ru, -b * rv) / det, a.mul_add(rv, -b * ru) / det);
            let length = step.magnitude();
            if length > self.reach / 2.0 {
                step *= self.reach / 2.0 / length;
            }
            let (next, clamped) = clamp(q + step);
            if at_edge && clamped && (next - q).magnitude() <= 1e-9 * self.reach {
                return Ok(None);
            }
            q = next;
            at_edge = clamped;
        }
        Err(lost().into())
    }
}

/// Which of `faces` the line through `middle` along `along` meets nearest
/// `middle`, the point it meets the face's surface at, and the face's
/// outward normal there.
fn landing(
    model: &Model,
    faces: &[Shape],
    middle: Point,
    along: Direction,
) -> Result<(Shape, Point, Vector), String> {
    let mut best: Option<(f64, &Shape)> = None;
    for face in faces {
        let Ok(drawn) = triangulate_face(model, face, Deflection::default(), tol()) else {
            continue;
        };
        for t in &drawn.triangles {
            let [a, b, c] = t.map(|i| drawn.positions[i as usize]);
            if let Some(hit) = super::sweep::ray_triangle(middle, along, a, b, c)
                && best.is_none_or(|(d, _)| hit.abs() < d.abs())
            {
                best = Some((hit, face));
            }
        }
    }
    let Some((hit, face)) = best else {
        return Err(
            "the middle of the sketch does not land on the picked faces along its plane's normal"
                .into(),
        );
    };
    let surface = placed_surface(model, face)?;
    let n = along.vector();
    // From the drawn hit, onto the surface: the line met with the tangent
    // plane at the surface's nearest point, until it settles.
    let mut q = middle + n * hit;
    let mut normal = n;
    for _ in 0..32 {
        let found = project_on_surface(&surface, q, 24, tol()).map_err(|e| e.to_string())?;
        let (u, v) = found.parameters;
        let (su, sv) = surface.d1_at(u, v, tol()).map_err(|e| e.to_string())?;
        let cross = su.cross(sv);
        if cross.magnitude() <= 0.0 {
            return Err("the middle of the sketch lands on a pole of the face's surface".into());
        }
        normal = cross * (1.0 / cross.magnitude());
        let facing = n.dot(normal);
        if facing.abs() < 1e-6 {
            return Err(
                "the sketch's plane stands square to the face where its middle lands".into(),
            );
        }
        let next = middle + n * ((found.point - middle).dot(normal) / facing);
        let moved = next.distance(q);
        q = next;
        if moved <= tol().confusion() {
            break;
        }
    }
    let found = project_on_surface(&surface, q, 24, tol()).map_err(|e| e.to_string())?;
    if face.orientation() == Orientation::Reversed {
        normal = -normal;
    }
    Ok((face.clone(), found.point, normal))
}

/// Points along `edge`, one every quarter millimetre or so, so a narrow
/// face it crosses is found.
fn samples_of(model: &Model, edge: &Shape) -> Result<Vec<Point>, String> {
    let (curve, range) = curve_of(model, edge)?;
    let at = |share: f64| {
        curve
            .point_at((range.1 - range.0).mul_add(share, range.0), tol())
            .map_err(|e| format!("a wrapped curve does not evaluate: {e}"))
    };
    let coarse = (0..=16)
        .map(|k| at(f64::from(k) / 16.0))
        .collect::<Result<Vec<_>, _>>()?;
    let length: f64 = coarse.windows(2).map(|w| w[0].distance(w[1])).sum();
    let count = ((length / 0.25).ceil() as usize).clamp(16, 2000);
    (0..=count).map(|k| at(k as f64 / count as f64)).collect()
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

/// The surface `face` lies on, placed as the face is.
fn placed_surface(model: &Model, face: &Shape) -> Result<SurfaceGeometry, String> {
    let Some(NodeData::Face(data)) = model.node(face).map(|n| n.data()) else {
        return Err("a picked face holds no face data".into());
    };
    let placement = face.transform(model.datums()).map_err(|e| e.to_string())?;
    model
        .geometry()
        .surface(data.surface)
        .ok_or("a picked face's surface is missing")?
        .transformed(&placement, tol())
        .map_err(|e| e.to_string())
}

/// The cylinder or cone `face` lies on, and its frame, placed as the face
/// is; `None` for a curved surface that does not unroll.
fn surface_of(model: &Model, face: &Shape) -> Result<Option<(Kind, Frame)>, String> {
    match placed_surface(model, face)? {
        SurfaceGeometry::Cylinder(c) => {
            let c = c.cylinder();
            Ok(Some((Kind::Cylinder { radius: c.radius() }, c.frame())))
        }
        SurfaceGeometry::Cone(c) => {
            let c = c.cone();
            Ok(Some((
                Kind::Cone {
                    slope: c.half_angle().tan(),
                    reference_radius: c.reference_radius(),
                },
                c.frame(),
            )))
        }
        SurfaceGeometry::Plane(_) => Err("wrapping needs faces on a curved surface; a flat \
             face takes the sketch projected without wrapping"
            .into()),
        _ => Ok(None),
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
                for (k, c) in array(p).into_iter().enumerate() {
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
    Ok((middle, point(low).distance(point(high))))
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
