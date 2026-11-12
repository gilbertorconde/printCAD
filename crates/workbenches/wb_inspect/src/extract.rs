//! A fitted element as a solid primitive: the shape and placement a
//! modelling bench builds it from, at the element's fitted size and over
//! the extent its points cover, in the document's frame.

use glam::DVec3;
use kernel_api::{FittedShape, Placement, PrimitiveKind};

use crate::element::{Element, Span};
use crate::vec::unit;

type V = [f64; 3];

/// A direction square to `axis`; `None` when `axis` has none.
fn across(axis: DVec3) -> Option<DVec3> {
    let other = if axis.x.abs() < 0.9 {
        DVec3::X
    } else {
        DVec3::Y
    };
    unit(axis.cross(other))
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
    let no_direction = || "the element has no direction".to_string();
    let point = |p: V| -> V {
        std::array::from_fn(|i| (0..3).map(|k| rows[i][k] * p[k]).sum::<f64>() + rows[i][3])
    };
    let direction = |d: DVec3| -> Result<V, String> {
        let moved: V = std::array::from_fn(|i| (0..3).map(|k| rows[i][k] * d[k]).sum());
        Ok(unit(moved.into()).ok_or_else(no_direction)?.to_array())
    };
    let placed = |origin: DVec3, z: DVec3, x: DVec3| -> Result<Placement, String> {
        Ok(Placement {
            origin: point(origin.to_array()),
            x_axis: direction(x)?,
            z_axis: direction(z)?,
        })
    };
    let on_axis = |axis: V| -> Result<(DVec3, DVec3), String> {
        let axis = DVec3::from(axis);
        Ok((axis, across(axis).ok_or_else(no_direction)?))
    };
    let r = &element.result;
    match (&r.shape, r.span) {
        (FittedShape::Plane { origin, normal }, Span::Patch { u, across: [a, b] }) => {
            if thickness <= 0.0 {
                return Err("the slab's thickness is above zero".into());
            }
            let n = unit((*normal).into()).ok_or_else(no_direction)?;
            let u = DVec3::from(u);
            let v = n.cross(u);
            let corner = ((DVec3::from(*origin) + u * a[0]) + v * b[0]) + n * -thickness;
            Ok((
                PrimitiveKind::Box {
                    length: a[1] - a[0],
                    width: b[1] - b[0],
                    height: thickness,
                },
                placed(corner, n, u)?,
            ))
        }
        (
            FittedShape::Cylinder {
                point: p,
                axis,
                radius,
            },
            Span::Axis([t0, t1]),
        ) => {
            let (axis, x) = on_axis(*axis)?;
            Ok((
                PrimitiveKind::Cylinder {
                    radius: *radius,
                    height: t1 - t0,
                    angle_deg: 360.0,
                },
                placed(DVec3::from(*p) + axis * t0, axis, x)?,
            ))
        }
        (
            FittedShape::Cone {
                apex,
                axis,
                half_angle,
            },
            Span::Axis([t0, t1]),
        ) => {
            let (axis, x) = on_axis(*axis)?;
            let tan = half_angle.tan();
            Ok((
                PrimitiveKind::Cone {
                    radius1: t0.abs() * tan,
                    radius2: t1.abs() * tan,
                    height: t1 - t0,
                    angle_deg: 360.0,
                },
                placed(DVec3::from(*apex) + axis * t0, axis, x)?,
            ))
        }
        (FittedShape::Sphere { centre, radius }, _) => Ok((
            PrimitiveKind::Sphere {
                radius: *radius,
                angle1_deg: -90.0,
                angle2_deg: 90.0,
                angle3_deg: 360.0,
            },
            placed((*centre).into(), DVec3::Z, DVec3::X)?,
        )),
        (
            FittedShape::Torus {
                centre,
                axis,
                major,
                minor,
            },
            _,
        ) => {
            let (axis, x) = on_axis(*axis)?;
            Ok((
                PrimitiveKind::Torus {
                    radius1: *major,
                    radius2: *minor,
                    angle1_deg: -180.0,
                    angle2_deg: 180.0,
                    angle3_deg: 360.0,
                },
                placed((*centre).into(), axis, x)?,
            ))
        }
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
