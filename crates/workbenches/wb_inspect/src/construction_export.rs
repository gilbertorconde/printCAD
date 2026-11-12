//! Elements, section cuts and shadow outlines as construction geometry
//! where they stand, and that geometry written as STEP or IGES.

use crate::{
    InspectionWorkbench, construction, element, element::Element, making::element_at, section,
    shadow, vec,
};
use core_document::{BodyId, Document};
use glam::DVec3;

pub(crate) fn write_construction(
    kernel: &dyn kernel_api::KernelQueries,
    items: &[kernel_api::ConstructionItem],
    iges: bool,
) -> kernel_api::KernelResult<(Vec<u8>, Vec<String>)> {
    if iges {
        let written = kernel.write_construction_iges("Inspection", items)?;
        Ok((written.text.into_bytes(), written.warnings))
    } else {
        kernel
            .write_construction_step("Inspection", items)
            .map(|bytes| (bytes, Vec::new()))
    }
}

/// `shape` of `e` as construction geometry, in the scan's frame; an
/// element with no direction gives none.
pub(crate) fn construction_of(e: &Element) -> Vec<(String, kernel_api::ConstructionShape)> {
    construction_items(e).unwrap_or_default()
}

pub(crate) fn construction_items(
    e: &Element,
) -> Option<Vec<(String, kernel_api::ConstructionShape)>> {
    use kernel_api::{ConstructionShape as S, FittedShape as F};
    let name = e.name.clone();
    let unit = |a: [f64; 3]| vec::unit(a.into());
    let along = |at: [f64; 3], d: DVec3, t: f64| (DVec3::from(at) + d * t).to_array();
    let axis = |name: &str, at: [f64; 3], a: [f64; 3]| {
        Some((
            format!("{name} axis"),
            S::Axis {
                origin: at,
                z: unit(a)?.to_array(),
                x: vec::basis(a.into())?.0.to_array(),
            },
        ))
    };
    let span = match e.result.span {
        element::Span::Axis(t) => Some(t),
        _ => None,
    };
    Some(match (e.result.shape.clone(), e.result.span) {
        (F::Plane { origin, normal }, element::Span::Patch { u, across }) => vec![(
            name,
            S::Plane {
                origin,
                normal: unit(normal)?.to_array(),
                x_axis: u,
                u: across[0],
                v: across[1],
            },
        )],
        (F::Plane { origin, normal }, _) => vec![(
            name,
            S::Plane {
                origin,
                normal: unit(normal)?.to_array(),
                x_axis: vec::basis(normal.into())?.0.to_array(),
                u: [-10.0, 10.0],
                v: [-10.0, 10.0],
            },
        )],
        (
            F::Cylinder {
                point,
                axis: a,
                radius,
            },
            _,
        ) => {
            let d = unit(a)?;
            let [t0, t1] = span.unwrap_or([0.0, 0.0]);
            let mut out = vec![axis(&name, along(point, d, t0), d.to_array())?];
            for (end, t) in [("start", t0), ("end", t1)] {
                out.push((
                    format!("{name} {end}"),
                    S::Circle {
                        centre: along(point, d, t),
                        normal: d.to_array(),
                        radius,
                    },
                ));
            }
            out
        }
        (
            F::Cone {
                apex,
                axis: a,
                half_angle,
            },
            _,
        ) => {
            let d = unit(a)?;
            let mut out = vec![axis(&name, apex, d.to_array())?];
            if let Some([t0, t1]) = span {
                for (end, t) in [("start", t0), ("end", t1)] {
                    let radius = (t * half_angle.tan()).abs();
                    if radius > 0.0 {
                        out.push((
                            format!("{name} {end}"),
                            S::Circle {
                                centre: along(apex, d, t),
                                normal: d.to_array(),
                                radius,
                            },
                        ));
                    }
                }
            }
            out
        }
        (F::Sphere { centre, .. }, _) => vec![(name, S::Point(centre))],
        (
            F::Torus {
                centre,
                axis: a,
                major,
                ..
            },
            _,
        ) => vec![
            axis(&name, centre, a)?,
            (
                format!("{name} spine"),
                S::Circle {
                    centre,
                    normal: unit(a)?.to_array(),
                    radius: major,
                },
            ),
        ],
        (
            F::Circle {
                centre,
                normal,
                radius,
            },
            _,
        ) => vec![(
            name,
            S::Circle {
                centre,
                normal: unit(normal)?.to_array(),
                radius,
            },
        )],
        (F::Line { point, direction }, _) => {
            let d = unit(direction)?;
            let [t0, t1] = span.unwrap_or([-10.0, 10.0]);
            vec![(
                name,
                S::Polyline {
                    points: vec![along(point, d, t0), along(point, d, t1)],
                    closed: false,
                },
            )]
        }
    })
}

/// `shape` moved by a body's placement.
fn placed_shape(
    rows: [[f64; 4]; 4],
    shape: kernel_api::ConstructionShape,
) -> kernel_api::ConstructionShape {
    use kernel_api::ConstructionShape as S;
    let p = |v: [f64; 3]| -> [f64; 3] {
        std::array::from_fn(|r| {
            rows[r][0] * v[0] + rows[r][1] * v[1] + rows[r][2] * v[2] + rows[r][3]
        })
    };
    let d = |v: [f64; 3]| -> [f64; 3] {
        std::array::from_fn(|r| rows[r][0] * v[0] + rows[r][1] * v[1] + rows[r][2] * v[2])
    };
    match shape {
        S::Point(a) => S::Point(p(a)),
        S::Axis { origin, z, x } => S::Axis {
            origin: p(origin),
            z: d(z),
            x: d(x),
        },
        S::Polyline { points, closed } => S::Polyline {
            points: points.into_iter().map(p).collect(),
            closed,
        },
        S::Circle {
            centre,
            normal,
            radius,
        } => S::Circle {
            centre: p(centre),
            normal: d(normal),
            radius,
        },
        S::Plane {
            origin,
            normal,
            x_axis,
            u,
            v,
        } => S::Plane {
            origin: p(origin),
            normal: d(normal),
            x_axis: d(x_axis),
            u,
            v,
        },
    }
}

impl InspectionWorkbench {
    /// Every element, constructed element, section cut and shadow outline
    /// (of `only` when given) as construction geometry where it stands.
    pub(crate) fn construction_items(
        &self,
        document: &Document,
        only: Option<BodyId>,
    ) -> Vec<kernel_api::ConstructionItem> {
        let mut nodes: Vec<_> = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.visible)
            .collect();
        nodes.sort_by_key(|(id, n)| (n.seq, **id));
        let mut out = Vec::new();
        let mut push = |rows, name: String, shape| {
            out.push(kernel_api::ConstructionItem {
                name,
                shape: placed_shape(rows, shape),
            })
        };
        for (id, node) in nodes {
            let kind = node.workbench_id.as_str();
            if kind == element::KIND || kind == construction::KIND {
                let Some(e) = element_at(document, *id) else {
                    continue;
                };
                if only.is_some_and(|b| b != e.scan) {
                    continue;
                }
                let rows = document.body_placement(e.scan).rows();
                for (name, shape) in construction_of(&e) {
                    push(rows, name, shape);
                }
            } else if kind == section::KIND {
                let Some((s, cuts, _)) = self.cuts_of(document, *id) else {
                    continue;
                };
                if only.is_some_and(|b| b != s.scan) {
                    continue;
                }
                let rows = document.body_placement(s.scan).rows();
                for c in cuts.iter() {
                    for (k, chain) in c.chains.iter().enumerate() {
                        push(
                            rows,
                            format!("{} {:+.2} #{}", node.name, c.offset, k + 1),
                            kernel_api::ConstructionShape::Polyline {
                                points: chain.points.clone(),
                                closed: chain.closed,
                            },
                        );
                    }
                }
            } else if kind == shadow::KIND {
                let Some(s) = document.feature_as::<shadow::Shadow>(*id) else {
                    continue;
                };
                let Some(made) = s.made.as_ref().filter(|_| only.is_none_or(|b| b == s.scan))
                else {
                    continue;
                };
                let p = made.plane;
                for (k, wire) in made.outline.iter().enumerate() {
                    let points = shadow::samples(wire)
                        .map(|at| shadow::on_plane(&p, at))
                        .collect();
                    // The outline is in the document's frame already.
                    push(
                        core_document::BodyPlacement::default().rows(),
                        format!("{} #{}", node.name, k + 1),
                        kernel_api::ConstructionShape::Polyline {
                            points,
                            closed: true,
                        },
                    );
                }
            }
        }
        out
    }
}
