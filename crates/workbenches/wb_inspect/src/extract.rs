//! A fitted element as a solid primitive: the shape and placement a
//! modelling bench builds it from, at the element's fitted size and over
//! the extent its points cover, in the document's frame.

use kernel_api::{FittedShape, Placement, PrimitiveKind};

use crate::element::{Element, Span};

type V = [f64; 3];

fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: V, s: f64) -> V {
    a.map(|c| c * s)
}

fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(a: V) -> V {
    let len = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-300);
    scale(a, 1.0 / len)
}

/// A direction square to `axis`.
fn across(axis: V) -> V {
    let other = if axis[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    unit(cross(axis, other))
}

/// The primitive `element` stands for, in the frame `rows` (the scan's
/// placement) puts its scan in: a plane as a slab `thickness` thick under
/// its fitted patch (against its normal), a cylinder or cone over the
/// length its points cover, a sphere or torus whole.
pub fn primitive(
    element: &Element,
    rows: &[[f64; 4]; 4],
    thickness: f64,
) -> Result<(PrimitiveKind, Placement), String> {
    let point = |p: V| -> V {
        std::array::from_fn(|i| (0..3).map(|k| rows[i][k] * p[k]).sum::<f64>() + rows[i][3])
    };
    let direction = |d: V| -> V {
        unit(std::array::from_fn(|i| {
            (0..3).map(|k| rows[i][k] * d[k]).sum()
        }))
    };
    let placed = |origin: V, z: V, x: V| Placement {
        origin: point(origin),
        x_axis: direction(x),
        z_axis: direction(z),
    };
    let r = &element.result;
    match (&r.shape, r.span) {
        (FittedShape::Plane { origin, normal }, Span::Patch { u, across: [a, b] }) => {
            if thickness <= 0.0 {
                return Err("the slab's thickness is above zero".into());
            }
            let n = unit(*normal);
            let v = cross(n, u);
            let corner = add(
                add(add(*origin, scale(u, a[0])), scale(v, b[0])),
                scale(n, -thickness),
            );
            Ok((
                PrimitiveKind::Box {
                    length: a[1] - a[0],
                    width: b[1] - b[0],
                    height: thickness,
                },
                placed(corner, n, u),
            ))
        }
        (
            FittedShape::Cylinder {
                point: p,
                axis,
                radius,
            },
            Span::Axis([t0, t1]),
        ) => Ok((
            PrimitiveKind::Cylinder {
                radius: *radius,
                height: t1 - t0,
                angle_deg: 360.0,
            },
            placed(add(*p, scale(*axis, t0)), *axis, across(*axis)),
        )),
        (
            FittedShape::Cone {
                apex,
                axis,
                half_angle,
            },
            Span::Axis([t0, t1]),
        ) => {
            let tan = half_angle.tan();
            Ok((
                PrimitiveKind::Cone {
                    radius1: t0.abs() * tan,
                    radius2: t1.abs() * tan,
                    height: t1 - t0,
                    angle_deg: 360.0,
                },
                placed(add(*apex, scale(*axis, t0)), *axis, across(*axis)),
            ))
        }
        (FittedShape::Sphere { centre, radius }, _) => Ok((
            PrimitiveKind::Sphere {
                radius: *radius,
                angle1_deg: -90.0,
                angle2_deg: 90.0,
                angle3_deg: 360.0,
            },
            placed(*centre, [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        )),
        (
            FittedShape::Torus {
                centre,
                axis,
                major,
                minor,
            },
            _,
        ) => Ok((
            PrimitiveKind::Torus {
                radius1: *major,
                radius2: *minor,
                angle1_deg: -180.0,
                angle2_deg: 180.0,
                angle3_deg: 360.0,
            },
            placed(*centre, *axis, across(*axis)),
        )),
        (FittedShape::Circle { .. } | FittedShape::Line { .. }, _) => Err(
            "only a fitted surface becomes a solid: a plane, cylinder, cone, sphere or torus"
                .into(),
        ),
        _ => Err("the element has no extent to build over".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::Fitted;
    use core_document::BodyId;
    use kernel_api::FitKind;

    fn element(shape: FittedShape, span: Span) -> Element {
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

    /// A plane's patch becomes a slab under it, a cone keeps its radii at
    /// the ends of its span, and a scan placed off the origin carries its
    /// element with it.
    #[test]
    fn an_element_becomes_the_primitive_it_stands_for() {
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let plane = element(
            FittedShape::Plane {
                origin: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
            },
            Span::Patch {
                u: [1.0, 0.0, 0.0],
                across: [[0.0, 4.0], [0.0, 3.0]],
            },
        );
        let (kind, at) = primitive(&plane, &identity, 2.0).unwrap();
        assert_eq!(
            kind,
            PrimitiveKind::Box {
                length: 4.0,
                width: 3.0,
                height: 2.0
            }
        );
        assert_eq!((at.origin, at.z_axis), ([0.0, 0.0, -2.0], [0.0, 0.0, 1.0]));

        let cone = element(
            FittedShape::Cone {
                apex: [0.0; 3],
                axis: [0.0, 0.0, 1.0],
                half_angle: std::f64::consts::FRAC_PI_4,
            },
            Span::Axis([2.0, 5.0]),
        );
        let mut moved = identity;
        moved[0][3] = 10.0;
        let (kind, at) = primitive(&cone, &moved, 2.0).unwrap();
        let PrimitiveKind::Cone {
            radius1,
            radius2,
            height,
            ..
        } = kind
        else {
            panic!("{kind:?}")
        };
        assert!((radius1 - 2.0).abs() < 1e-12 && (radius2 - 5.0).abs() < 1e-12 && height == 3.0);
        assert_eq!(at.origin, [10.0, 0.0, 2.0]);
        assert!(
            primitive(
                &element(
                    FittedShape::Line {
                        point: [0.0; 3],
                        direction: [1.0, 0.0, 0.0]
                    },
                    Span::Axis([0.0, 1.0])
                ),
                &identity,
                2.0
            )
            .is_err()
        );
    }
}
