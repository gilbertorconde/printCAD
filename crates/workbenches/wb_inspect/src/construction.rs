//! Constructed elements: points, lines, planes and circles made from other
//! elements (a midpoint, two planes' intersection, a plane through three
//! points, a section of a cylinder). A construction keeps only what it is
//! made from and how; its shape is worked out whenever it is asked, so it
//! follows its sources when they are refitted.
//!
//! A constructed point is carried as a sphere of radius zero, which every
//! reader of a shape takes as its centre.

use core_document::{
    BodyId, DocumentError, DocumentResult, FeatureId, WorkbenchFeature, WorkbenchId,
};
use kernel_api::{FitKind, FittedShape};
use serde::{Deserialize, Serialize};

use crate::element::{Element, Fitted, Span, add, basis, cross, dot, scale, span_of, sub};

/// The feature kind of every constructed element.
pub const KIND: &str = "wb.inspect.construction";

/// A plane cut through a cylinder further than this from square, in
/// degrees, makes an ellipse, not a circle.
pub const SECTION_LEAN_DEG: f64 = 5.0;

/// What a construction makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Made {
    Point,
    Line,
    Plane,
    Circle,
}

impl Made {
    pub const ALL: [Made; 4] = [Made::Point, Made::Line, Made::Plane, Made::Circle];

    pub fn word(self) -> &'static str {
        match self {
            Made::Point => "Point",
            Made::Line => "Line",
            Made::Plane => "Plane",
            Made::Circle => "Circle",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Made::Point => "datum-point",
            Made::Line => "datum-line",
            Made::Plane => "datum-plane",
            Made::Circle => "circle",
        }
    }

    /// What each way of making it takes, as the tool's tip says.
    pub fn takes(self) -> &'static str {
        match self {
            Made::Point => {
                "two centres (their midpoint), or an axis and a plane (where it crosses)"
            }
            Made::Line => "two centres (the line through them), or two planes (where they meet)",
            Made::Plane => {
                "three centres (the plane through them), two planes (the one midway), or one \
                 plane (offset from it)"
            }
            Made::Circle => "a plane and a cylinder or sphere (where the plane cuts it)",
        }
    }
}

/// One constructed element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Construction {
    pub name: String,
    /// The scan its sources were fitted on.
    pub scan: BodyId,
    pub made: Made,
    /// The elements it is made from, in the order picked.
    pub from: Vec<FeatureId>,
    /// A plane made from one plane lies this far along its normal.
    #[serde(default)]
    pub offset: f64,
}

impl WorkbenchFeature for Construction {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(KIND)
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    fn from_json(value: &serde_json::Value) -> DocumentResult<Self> {
        serde_json::from_value(value.clone()).map_err(DocumentError::Serialization)
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        self.from.clone()
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// A source as a construction reads it.
enum Source {
    Centre([f64; 3]),
    Axis([f64; 3], [f64; 3]),
    Plane {
        origin: [f64; 3],
        normal: [f64; 3],
        span: Span,
    },
    Round {
        shape: FittedShape,
    },
}

fn sources(e: &Element) -> Vec<Source> {
    match e.result.shape {
        FittedShape::Sphere { centre, .. } => {
            vec![
                Source::Centre(centre),
                Source::Round {
                    shape: e.result.shape.clone(),
                },
            ]
        }
        FittedShape::Circle { centre, normal, .. }
        | FittedShape::Torus {
            centre,
            axis: normal,
            ..
        } => {
            vec![Source::Centre(centre), Source::Axis(centre, unit(normal))]
        }
        FittedShape::Cylinder { point, axis, .. } => vec![
            Source::Axis(point, unit(axis)),
            Source::Round {
                shape: e.result.shape.clone(),
            },
        ],
        FittedShape::Cone { apex, axis, .. } => vec![Source::Axis(apex, unit(axis))],
        FittedShape::Line { point, direction } => {
            vec![Source::Axis(point, unit(direction))]
        }
        FittedShape::Plane { origin, normal } => vec![Source::Plane {
            origin,
            normal: unit(normal),
            span: e.result.span,
        }],
    }
}

fn centre(e: &Element) -> Option<[f64; 3]> {
    sources(e).into_iter().find_map(|s| match s {
        Source::Centre(c) => Some(c),
        _ => None,
    })
}

fn axis(e: &Element) -> Option<([f64; 3], [f64; 3])> {
    sources(e).into_iter().find_map(|s| match s {
        Source::Axis(p, d) => Some((p, d)),
        _ => None,
    })
}

fn plane(e: &Element) -> Option<([f64; 3], [f64; 3], Span)> {
    sources(e).into_iter().find_map(|s| match s {
        Source::Plane {
            origin,
            normal,
            span,
        } => Some((origin, normal, span)),
        _ => None,
    })
}

fn round(e: &Element) -> Option<FittedShape> {
    sources(e).into_iter().find_map(|s| match s {
        Source::Round { shape } => Some(shape),
        _ => None,
    })
}

/// A plane's measured patch, as points: its corners.
fn patch_corners(origin: [f64; 3], normal: [f64; 3], span: Span) -> Vec<[f64; 3]> {
    match span {
        Span::Patch { u, across } => {
            let v = cross(normal, u);
            let mut out = Vec::new();
            for a in across[0] {
                for b in across[1] {
                    out.push(add(origin, add(scale(u, a), scale(v, b))));
                }
            }
            out
        }
        _ => vec![origin],
    }
}

/// The shape `c` makes from `from`, as an element its dimensions and
/// drawing read like a fitted one.
pub fn build(c: &Construction, from: &[Element]) -> Result<Element, String> {
    let need = |what: &str| {
        format!(
            "a constructed {} takes {}",
            c.made.word().to_lowercase(),
            what
        )
    };
    let shaped = |shape: FittedShape, span: Span| Element {
        method: Default::default(),
        name: c.name.clone(),
        scan: c.scan,
        kind: match c.made {
            Made::Point => FitKind::Sphere,
            Made::Line => FitKind::Line,
            Made::Plane => FitKind::Plane,
            Made::Circle => FitKind::Circle,
        },
        cutoff: None,
        region: Vec::new(),
        revision: 0,
        result: Fitted {
            zone: None,
            shape,
            sigma: 0.0,
            form: (0.0, 0.0),
            used: 0,
            points: 0,
            span,
        },
    };
    let point = |p: [f64; 3]| {
        shaped(
            FittedShape::Sphere {
                centre: p,
                radius: 0.0,
            },
            Span::None,
        )
    };
    let line_through = |p: [f64; 3], q: [f64; 3]| -> Result<Element, String> {
        let d = sub(q, p);
        let len = dot(d, d).sqrt();
        if len < 1e-9 {
            return Err("the two centres are at one spot".into());
        }
        Ok(shaped(
            FittedShape::Line {
                point: p,
                direction: scale(d, 1.0 / len),
            },
            Span::Axis([0.0, len]),
        ))
    };
    let plane_with = |origin: [f64; 3], normal: [f64; 3], corners: &[[f64; 3]]| {
        let shape = FittedShape::Plane { origin, normal };
        let on: Vec<[f64; 3]> = corners
            .iter()
            .map(|p| sub(*p, scale(normal, dot(sub(*p, origin), normal))))
            .collect();
        let span = span_of(&shape, &on);
        shaped(shape, span)
    };
    match (c.made, from) {
        (Made::Point, [a, b]) => {
            if let (Some(p), Some(q)) = (centre(a), centre(b)) {
                return Ok(point(scale(add(p, q), 0.5)));
            }
            let (ax, pl) = match (axis(a), plane(b), axis(b), plane(a)) {
                (Some(ax), Some(pl), _, _) | (_, _, Some(ax), Some(pl)) => (ax, pl),
                _ => return Err(need("two centres, or an axis and a plane")),
            };
            let ((p, d), (o, n, _)) = (ax, pl);
            let along = dot(d, n);
            if along.abs() < 1e-6 {
                return Err("the axis runs along the plane and never crosses it".into());
            }
            Ok(point(add(p, scale(d, dot(sub(o, p), n) / along))))
        }
        (Made::Line, [a, b]) => {
            if let (Some(p), Some(q)) = (centre(a), centre(b)) {
                return line_through(p, q);
            }
            let ((oa, na, sa), (ob, nb, sb)) = match (plane(a), plane(b)) {
                (Some(pa), Some(pb)) => (pa, pb),
                _ => return Err(need("two centres, or two planes")),
            };
            let d = cross(na, nb);
            let len = dot(d, d).sqrt();
            if len < 1e-6 {
                return Err("the planes are parallel and never meet".into());
            }
            let d = scale(d, 1.0 / len);
            // The point on both planes nearest the first's origin.
            let (da, db) = (dot(na, oa), dot(nb, ob));
            let k = dot(na, nb);
            let den = 1.0 - k * k;
            let (ca, cb) = ((da - db * k) / den, (db - da * k) / den);
            let p = add(scale(na, ca), scale(nb, cb));
            // As long as the two patches reach along it.
            let ts: Vec<f64> = patch_corners(oa, na, sa)
                .into_iter()
                .chain(patch_corners(ob, nb, sb))
                .map(|q| dot(sub(q, p), d))
                .collect();
            let lo = ts.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = ts.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            Ok(shaped(
                FittedShape::Line {
                    point: p,
                    direction: d,
                },
                Span::Axis([lo, hi]),
            ))
        }
        (Made::Plane, [a, b, e]) => {
            let (Some(p), Some(q), Some(r)) = (centre(a), centre(b), centre(e)) else {
                return Err(need("three centres, two planes or one plane"));
            };
            let n = cross(sub(q, p), sub(r, p));
            let len = dot(n, n).sqrt();
            if len < 1e-9 {
                return Err("the three centres lie on one line".into());
            }
            Ok(plane_with(p, scale(n, 1.0 / len), &[p, q, r]))
        }
        (Made::Plane, [a, b]) => {
            let ((oa, na, sa), (ob, nb, sb)) = match (plane(a), plane(b)) {
                (Some(pa), Some(pb)) => (pa, pb),
                _ => return Err(need("three centres, two planes or one plane")),
            };
            let nb = if dot(na, nb) < 0.0 {
                scale(nb, -1.0)
            } else {
                nb
            };
            let n = add(na, nb);
            let n = scale(n, 1.0 / dot(n, n).sqrt().max(1e-12));
            let o = scale(add(oa, ob), 0.5);
            let corners: Vec<[f64; 3]> = patch_corners(oa, na, sa)
                .into_iter()
                .chain(patch_corners(ob, nb, sb))
                .collect();
            Ok(plane_with(o, n, &corners))
        }
        (Made::Plane, [a]) => {
            let Some((o, n, span)) = plane(a) else {
                return Err(need("three centres, two planes or one plane"));
            };
            let o2 = add(o, scale(n, c.offset));
            Ok(shaped(
                FittedShape::Plane {
                    origin: o2,
                    normal: n,
                },
                span,
            ))
        }
        (Made::Circle, [a, b]) => {
            let ((o, n, _), shape) = match (plane(a), round(b), plane(b), round(a)) {
                (Some(pl), Some(r), _, _) | (_, _, Some(pl), Some(r)) => (pl, r),
                _ => return Err(need("a plane and a cylinder or sphere")),
            };
            match shape {
                FittedShape::Sphere { centre, radius } => {
                    let h = dot(sub(centre, o), n);
                    if h.abs() >= radius {
                        return Err("the plane misses the sphere".into());
                    }
                    Ok(shaped(
                        FittedShape::Circle {
                            centre: sub(centre, scale(n, h)),
                            normal: n,
                            radius: (radius * radius - h * h).sqrt(),
                        },
                        Span::None,
                    ))
                }
                FittedShape::Cylinder {
                    point,
                    axis,
                    radius,
                } => {
                    let d = unit(axis);
                    let along = dot(d, n);
                    if along.abs() < SECTION_LEAN_DEG.to_radians().cos() {
                        return Err(format!(
                            "the plane leans more than {SECTION_LEAN_DEG}° from square to the \
                             cylinder: its cut is an ellipse"
                        ));
                    }
                    let centre = add(point, scale(d, dot(sub(o, point), n) / along));
                    Ok(shaped(
                        FittedShape::Circle {
                            centre,
                            normal: d,
                            radius,
                        },
                        Span::None,
                    ))
                }
                _ => Err(need("a plane and a cylinder or sphere")),
            }
        }
        (made, _) => Err(format!(
            "a constructed {} takes {}",
            made.word().to_lowercase(),
            made.takes()
        )),
    }
}

/// The lines a constructed point draws: a small cross.
pub fn cross_at(p: [f64; 3], size: f64) -> [([f64; 3], [f64; 3]); 3] {
    let (u, v) = basis([0.0, 0.0, 1.0]);
    let w = [0.0, 0.0, 1.0];
    [u, v, w].map(|d| (sub(p, scale(d, size)), add(p, scale(d, size))))
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    scale(v, 1.0 / dot(v, v).sqrt().max(1e-12))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fitted(shape: FittedShape) -> Element {
        let span = match shape {
            FittedShape::Plane { origin, normal } => {
                let (u, v) = basis(normal);
                let corners = [-1.0, 1.0].map(|k| add(origin, add(scale(u, k), scale(v, k))));
                span_of(&shape, &corners)
            }
            _ => Span::None,
        };
        Element {
            method: Default::default(),
            name: "e".into(),
            scan: BodyId(uuid::Uuid::nil()),
            kind: FitKind::Plane,
            cutoff: None,
            region: Vec::new(),
            revision: 0,
            result: Fitted {
                zone: None,
                shape,
                sigma: 0.0,
                form: (0.0, 0.0),
                used: 0,
                points: 0,
                span,
            },
        }
    }

    fn make(made: Made, from: &[Element], offset: f64) -> Result<FittedShape, String> {
        let c = Construction {
            name: "c".into(),
            scan: from[0].scan,
            made,
            from: Vec::new(),
            offset,
        };
        build(&c, from).map(|e| e.result.shape)
    }

    /// Each construction at its closed form, a dimension reading one as it
    /// reads a fitted element, and the cases that cannot be built refused.
    #[test]
    fn constructions_make_their_closed_forms() {
        let plane = |o: [f64; 3], n: [f64; 3]| {
            fitted(FittedShape::Plane {
                origin: o,
                normal: n,
            })
        };
        let ball = |c: [f64; 3]| {
            fitted(FittedShape::Sphere {
                centre: c,
                radius: 5.0,
            })
        };
        let (floor, roof) = (
            plane([0.0; 3], [0.0, 0.0, 1.0]),
            plane([0.0, 0.0, 10.0], [0.0, 0.0, -1.0]),
        );
        let wall = plane([4.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        let bore = |axis: [f64; 3]| {
            fitted(FittedShape::Cylinder {
                point: [0.0; 3],
                axis,
                radius: 3.0,
            })
        };
        let close = |a: [f64; 3], b: [f64; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-9);
        let centre = |s: FittedShape| match s {
            FittedShape::Sphere {
                centre,
                radius: 0.0,
            } => centre,
            other => panic!("{other:?}"),
        };

        let mid = make(Made::Point, &[ball([0.0; 3]), ball([2.0, 4.0, 6.0])], 0.0).unwrap();
        assert!(close(centre(mid), [1.0, 2.0, 3.0]));
        let hit = make(Made::Point, &[bore([1.0, 0.0, 0.0]), wall.clone()], 0.0).unwrap();
        assert!(close(centre(hit), [4.0, 0.0, 0.0]));
        let FittedShape::Line { point, direction } =
            make(Made::Line, &[floor.clone(), wall.clone()], 0.0).unwrap()
        else {
            panic!()
        };
        assert!(close(point, [4.0, 0.0, 0.0]) && direction[1].abs() > 1.0 - 1e-12);

        let half = make(Made::Plane, &[floor.clone(), roof.clone()], 0.0).unwrap();
        assert!(
            matches!(half, FittedShape::Plane { origin, .. } if (origin[2] - 5.0).abs() < 1e-9)
        );
        let raised = make(Made::Plane, std::slice::from_ref(&floor), 3.0).unwrap();
        assert!(
            matches!(raised, FittedShape::Plane { origin, .. } if (origin[2] - 3.0).abs() < 1e-9)
        );
        let three = [[0.0, 0.0, 2.0], [1.0, 0.0, 2.0], [0.0, 1.0, 2.0]].map(ball);
        let FittedShape::Plane { origin, normal } = make(Made::Plane, &three, 0.0).unwrap() else {
            panic!()
        };
        assert!((origin[2] - 2.0).abs() < 1e-9 && normal[2].abs() > 1.0 - 1e-12);

        let cut = plane([0.0, 0.0, 2.0], [0.0, 0.0, 1.0]);
        let ring = make(Made::Circle, &[cut.clone(), ball([0.0; 3])], 0.0).unwrap();
        assert!(
            matches!(ring, FittedShape::Circle { radius, .. } if (radius - 21f64.sqrt()).abs() < 1e-9)
        );
        let section = make(Made::Circle, &[bore([0.0, 0.0, 1.0]), cut.clone()], 0.0).unwrap();
        assert!(matches!(section, FittedShape::Circle { centre, radius, .. }
            if close(centre, [0.0, 0.0, 2.0]) && radius == 3.0));
        let leaning = bore([0.0, 0.2, 1.0]);
        assert!(
            make(Made::Circle, &[cut, leaning], 0.0).is_err(),
            "11° is an ellipse"
        );
        assert!(
            make(Made::Line, &[floor.clone(), roof.clone()], 0.0).is_err(),
            "parallel"
        );
        assert!(make(Made::Plane, std::slice::from_ref(&floor), 0.0).is_ok());
        assert!(make(Made::Circle, std::slice::from_ref(&floor), 0.0).is_err());

        let c = Construction {
            name: "Plane 1".into(),
            scan: floor.scan,
            made: Made::Plane,
            from: Vec::new(),
            offset: 0.0,
        };
        let midway = build(&c, &[floor.clone(), roof]).unwrap();
        let d =
            crate::dimension::read(crate::dimension::DimKind::Distance, &[midway, floor]).unwrap();
        assert!((d.value - 5.0).abs() < 1e-9);
    }
}
