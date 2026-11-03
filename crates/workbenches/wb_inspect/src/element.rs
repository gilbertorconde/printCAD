//! A fitted element: the shape a part of a scan was fitted to, what it
//! rests on, and the lines that draw it, all in the scan's own frame.

use core_document::{BodyId, DocumentError, DocumentResult, WorkbenchFeature, WorkbenchId};
use kernel_api::{FitKind, FittedShape, TriMesh};
use serde::{Deserialize, Serialize};

/// The feature kind of every Inspection element.
pub const KIND: &str = "wb.inspect";

/// One element of an inspection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub name: String,
    /// The scan it was fitted on.
    pub scan: BodyId,
    pub kind: FitKind,
    /// Points farther than this many sigmas were left out; `None` kept all.
    #[serde(default)]
    pub cutoff: Option<f64>,
    /// How the shape was fitted to the points kept.
    #[serde(default)]
    pub method: kernel_api::FitMethod,
    /// The scan's vertices it was fitted to, as runs `[start, end)`.
    pub region: Vec<[u32; 2]>,
    /// The scan mesh's revision those vertices belong to.
    pub revision: u64,
    pub result: Fitted,
}

/// What a fit came to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fitted {
    pub shape: FittedShape,
    pub sigma: f64,
    /// Signed extremes of the points used: their difference is the form.
    pub form: (f64, f64),
    pub used: usize,
    pub points: usize,
    /// Where the points used lie along the shape, to draw it no larger
    /// than measured: along the axis (cylinder, cone, line) or, for a
    /// plane, across it in two directions square to its normal.
    pub span: Span,
    /// The narrowest zone of its kind holding the points used, when the
    /// kind has one: the form a tolerance reads.
    #[serde(default)]
    pub zone: Option<FormZone>,
}

/// A minimum zone's width, and whether it is certified the narrowest.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FormZone {
    pub width: f64,
    pub optimal: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Span {
    None,
    Axis([f64; 2]),
    Patch { u: [f64; 3], across: [[f64; 2]; 2] },
}

impl WorkbenchFeature for Element {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(KIND)
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    fn from_json(value: &serde_json::Value) -> DocumentResult<Self> {
        serde_json::from_value(value.clone()).map_err(DocumentError::Serialization)
    }

    fn dependencies(&self) -> Vec<core_document::FeatureId> {
        Vec::new()
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// The word a kind is named by.
pub fn kind_word(kind: FitKind) -> &'static str {
    match kind {
        FitKind::Plane => "Plane",
        FitKind::Sphere => "Sphere",
        FitKind::Cylinder => "Cylinder",
        FitKind::Cone => "Cone",
        FitKind::Torus => "Torus",
        FitKind::Circle => "Circle",
        FitKind::Line => "Line",
    }
}

pub fn icon(kind: FitKind) -> &'static str {
    match kind {
        FitKind::Plane => "datum-plane",
        FitKind::Sphere => "additive-sphere",
        FitKind::Cylinder => "additive-cylinder",
        FitKind::Cone => "additive-cone",
        FitKind::Torus => "additive-torus",
        FitKind::Circle => "circle",
        FitKind::Line => "datum-line",
    }
}

/// The vertices flagged in `marked`, as runs.
pub fn runs(marked: &[bool]) -> Vec<[u32; 2]> {
    let mut out: Vec<[u32; 2]> = Vec::new();
    for (i, on) in marked.iter().enumerate() {
        if !on {
            continue;
        }
        let i = i as u32;
        match out.last_mut() {
            Some(run) if run[1] == i => run[1] = i + 1,
            _ => out.push([i, i + 1]),
        }
    }
    out
}

/// The vertices of `runs`.
pub fn vertices(runs: &[[u32; 2]]) -> impl Iterator<Item = u32> + '_ {
    runs.iter().flat_map(|[a, b]| *a..*b)
}

impl Fitted {
    /// A closed primitive with the same extent as the drawn element.
    pub fn solid(&self, rows: [[f64; 4]; 4]) -> Option<kernel_api::MeshOperand> {
        use kernel_api::{MeshOperand, Placement, PrimitiveKind};
        let (kind, origin, axis) = match (&self.shape, self.span) {
            (FittedShape::Sphere { centre, radius }, _) if *radius > 0.0 => (
                PrimitiveKind::Sphere {
                    radius: *radius,
                    angle1_deg: -90.0,
                    angle2_deg: 90.0,
                    angle3_deg: 360.0,
                },
                *centre,
                [0.0, 0.0, 1.0],
            ),
            (
                FittedShape::Cylinder {
                    point,
                    axis,
                    radius,
                },
                Span::Axis([lo, hi]),
            ) if hi > lo => (
                PrimitiveKind::Cylinder {
                    radius: *radius,
                    height: hi - lo,
                    angle_deg: 360.0,
                },
                add(*point, scale(*axis, lo)),
                *axis,
            ),
            (
                FittedShape::Cone {
                    apex,
                    axis,
                    half_angle,
                },
                Span::Axis([lo, hi]),
            ) if hi > lo && lo * hi >= 0.0 => (
                PrimitiveKind::Cone {
                    radius1: lo.abs() * half_angle.tan(),
                    radius2: hi.abs() * half_angle.tan(),
                    height: hi - lo,
                    angle_deg: 360.0,
                },
                add(*apex, scale(*axis, lo)),
                *axis,
            ),
            _ => return None,
        };
        let moved = |p: [f64; 3], w: f64| {
            std::array::from_fn(|r| {
                rows[r][0] * p[0] + rows[r][1] * p[1] + rows[r][2] * p[2] + w * rows[r][3]
            })
        };
        Some(MeshOperand::Primitive {
            kind,
            placement: Placement {
                origin: moved(origin, 1.0),
                x_axis: moved(basis(axis).0, 0.0),
                z_axis: moved(axis, 0.0),
            },
        })
    }

    /// The size it is read by, and its unit: a diameter, a cone's
    /// included angle, a torus's ring and tube.
    pub fn size(&self) -> Option<String> {
        Some(match &self.shape {
            FittedShape::Sphere { radius, .. } if *radius == 0.0 => return None,
            FittedShape::Sphere { radius, .. }
            | FittedShape::Cylinder { radius, .. }
            | FittedShape::Circle { radius, .. } => format!("Ø {:.3} mm", 2.0 * radius),
            FittedShape::Cone { half_angle, .. } => {
                format!("{:.2}° included", 2.0 * half_angle.to_degrees())
            }
            FittedShape::Torus { major, minor, .. } => {
                format!("ring Ø {:.3}, tube Ø {:.3} mm", 2.0 * major, 2.0 * minor)
            }
            FittedShape::Plane { .. } | FittedShape::Line { .. } => return None,
        })
    }

    /// The smallest and largest diameter a round surface reaches.
    pub fn diameters(&self) -> Option<(f64, f64)> {
        match &self.shape {
            FittedShape::Sphere { radius, .. } if *radius == 0.0 => None,
            FittedShape::Sphere { radius, .. }
            | FittedShape::Cylinder { radius, .. }
            | FittedShape::Circle { radius, .. } => {
                Some((2.0 * (radius + self.form.0), 2.0 * (radius + self.form.1)))
            }
            _ => None,
        }
    }

    /// One line saying what it measured.
    pub fn summary(&self) -> String {
        let mut words = Vec::new();
        if let Some(size) = self.size() {
            words.push(size);
        }
        words.push(format!("σ {:.4}", self.sigma));
        words.push(format!("form {:.4} mm", self.form.1 - self.form.0));
        words.join(" · ")
    }

    /// The lines that draw it, in the scan's own frame.
    pub fn lines(&self) -> TriMesh {
        let mut mesh = TriMesh::default();
        let mut line = |a: [f64; 3], b: [f64; 3]| {
            let base = mesh.positions.len() as u32;
            for p in [a, b] {
                mesh.positions.push(p.map(|c| c as f32));
                mesh.normals.push([0.0, 0.0, 1.0]);
            }
            mesh.edges.extend([base, base + 1]);
        };
        let ring = |centre: [f64; 3],
                    normal: [f64; 3],
                    r: f64,
                    line: &mut dyn FnMut([f64; 3], [f64; 3])| {
            let (u, v) = basis(normal);
            let at = |k: usize| {
                let t = k as f64 * std::f64::consts::TAU / 48.0;
                add(centre, add(scale(u, r * t.cos()), scale(v, r * t.sin())))
            };
            for k in 0..48 {
                line(at(k), at(k + 1));
            }
        };
        match (&self.shape, self.span) {
            (FittedShape::Plane { origin, normal }, Span::Patch { u, across }) => {
                let v = cross(*normal, u);
                let corner = |a: f64, b: f64| add(*origin, add(scale(u, a), scale(v, b)));
                let [[u0, u1], [v0, v1]] = across;
                let c = [
                    corner(u0, v0),
                    corner(u1, v0),
                    corner(u1, v1),
                    corner(u0, v1),
                ];
                for k in 0..4 {
                    line(c[k], c[(k + 1) % 4]);
                }
            }
            // A point draws as a mark the same size at any zoom (the
            // bench's marks); its lines are only where a click picks it.
            (FittedShape::Sphere { centre, radius }, _) if *radius == 0.0 => {
                for (a, b) in crate::construction::cross_at(*centre, 0.0) {
                    line(a, b);
                }
            }
            (FittedShape::Sphere { centre, radius }, _) => {
                for n in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
                    ring(*centre, n, *radius, &mut line);
                }
            }
            (
                FittedShape::Circle {
                    centre,
                    normal,
                    radius,
                },
                _,
            ) => {
                ring(*centre, *normal, *radius, &mut line);
            }
            (
                FittedShape::Torus {
                    centre,
                    axis,
                    major,
                    ..
                },
                _,
            ) => {
                ring(*centre, *axis, *major, &mut line);
            }
            (
                FittedShape::Cylinder {
                    point,
                    axis,
                    radius,
                },
                Span::Axis([t0, t1]),
            ) => {
                for t in [t0, t1] {
                    ring(add(*point, scale(*axis, t)), *axis, *radius, &mut line);
                }
                let (u, v) = basis(*axis);
                for side in [u, v, scale(u, -1.0), scale(v, -1.0)] {
                    let at = |t| add(add(*point, scale(*axis, t)), scale(side, *radius));
                    line(at(t0), at(t1));
                }
            }
            (
                FittedShape::Cone {
                    apex,
                    axis,
                    half_angle,
                },
                Span::Axis([t0, t1]),
            ) => {
                for t in [t0, t1] {
                    ring(
                        add(*apex, scale(*axis, t)),
                        *axis,
                        t.abs() * half_angle.tan(),
                        &mut line,
                    );
                }
                line(add(*apex, scale(*axis, t0)), add(*apex, scale(*axis, t1)));
            }
            (FittedShape::Line { point, direction }, Span::Axis([t0, t1])) => {
                line(
                    add(*point, scale(*direction, t0)),
                    add(*point, scale(*direction, t1)),
                );
            }
            _ => {}
        }
        mesh
    }
}

/// Where `points` lie along a fitted shape: their extent along its axis,
/// or across a plane.
pub fn span_of(shape: &FittedShape, points: &[[f64; 3]]) -> Span {
    let along = |origin: [f64; 3], dir: [f64; 3]| {
        let ts = points.iter().map(|p| dot(sub(*p, origin), dir));
        let (lo, hi) = ts.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), t| {
            (lo.min(t), hi.max(t))
        });
        [lo, hi]
    };
    match shape {
        FittedShape::Cylinder { point, axis, .. }
        | FittedShape::Line {
            point,
            direction: axis,
        } => Span::Axis(along(*point, *axis)),
        FittedShape::Cone { apex, axis, .. } => Span::Axis(along(*apex, *axis)),
        FittedShape::Plane { origin, normal } => {
            let (u, v) = basis(*normal);
            Span::Patch {
                u,
                across: [along(*origin, u), along(*origin, v)],
            }
        }
        _ => Span::None,
    }
}

pub(crate) fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn scale(a: [f64; 3], k: f64) -> [f64; 3] {
    a.map(|c| c * k)
}

pub(crate) fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Two unit directions square to `n` and to each other.
pub(crate) fn basis(n: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let len = dot(n, n).sqrt().max(1e-12);
    let n = scale(n, 1.0 / len);
    let pick = if n[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let u = cross(n, pick);
    let u = scale(u, 1.0 / dot(u, u).sqrt());
    (u, cross(n, u))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Marked vertices pack into runs and unpack the same; a cylinder's
    /// span is its points' reach along the axis and it reads by its
    /// diameter and smallest and largest diameter.
    #[test]
    fn regions_pack_and_a_fit_reads_its_size() {
        let marked = [false, true, true, false, true, true, true];
        let packed = runs(&marked);
        assert_eq!(packed, vec![[1, 3], [4, 7]]);
        assert_eq!(vertices(&packed).collect::<Vec<_>>(), vec![1, 2, 4, 5, 6]);
        let shape = FittedShape::Cylinder {
            point: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            radius: 5.0,
        };
        let span = span_of(&shape, &[[5.0, 0.0, -2.0], [0.0, 5.0, 8.0]]);
        assert_eq!(span, Span::Axis([-2.0, 8.0]));
        let fitted = Fitted {
            zone: None,
            shape,
            sigma: 0.02,
            form: (-0.03, 0.05),
            used: 10,
            points: 10,
            span,
        };
        assert_eq!(fitted.size().as_deref(), Some("Ø 10.000 mm"));
        let (low, high) = fitted.diameters().unwrap();
        assert!((low - 9.94).abs() < 1e-12 && (high - 10.1).abs() < 1e-12);
        assert_eq!(fitted.lines().edges.len() / 2, 2 * 48 + 4);
    }
}
