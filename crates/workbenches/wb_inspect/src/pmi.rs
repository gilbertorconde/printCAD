//! Inspection's semantic annotations on the reference of a comparison.

use std::collections::{HashMap, HashSet};

use core_document::{
    BodyId, Document, PmiDatum, PmiDimension, PmiTolerance, SemanticPmi, WorkbenchFeature,
};
use kernel_api::FittedShape;

use crate::compare::Comparison;
use crate::dimension::{DimKind, Dimension};
use crate::element::{Element, Span, add, basis, cross, scale, vertices};
use crate::tolerance::{Characteristic, Tolerance};

pub(crate) fn of_body(document: &Document, body: BodyId) -> SemanticPmi {
    let mut nodes: Vec<_> = document.feature_tree().all_nodes().collect();
    nodes.sort_by_key(|(id, node)| (node.seq, **id));
    let scans: HashSet<_> = nodes
        .iter()
        .filter_map(|(id, node)| {
            if node.workbench_id.as_str() != crate::compare::KIND {
                return None;
            }
            let comparison = document.feature_as::<Comparison>(**id)?;
            (comparison.reference == body).then_some(comparison.scan)
        })
        .collect();
    let mut out = SemanticPmi::default();
    let mut labels = HashMap::new();
    for (id, node) in nodes {
        let Some(data) = document.feature_values(*id) else {
            continue;
        };
        match node.workbench_id.as_str() {
            crate::tolerance::KIND => {
                let Ok(t) = Tolerance::from_json(data) else {
                    continue;
                };
                if !scans.contains(&t.scan) {
                    continue;
                }
                let Some(element) = crate::element_at(document, t.element) else {
                    continue;
                };
                let mut datums = Vec::new();
                let refs = t.references();
                for reference in &refs {
                    let id = reference.element;
                    let Some(datum) = crate::element_at(document, id) else {
                        continue;
                    };
                    let label = labels.entry(id).or_insert_with(|| {
                        let label = datum_label(out.datums.len());
                        out.datums.push(PmiDatum {
                            label: label.clone(),
                            at: probes(document, &datum),
                        });
                        label
                    });
                    datums.push(label.clone());
                    if let Some(angle) = t
                        .angle_deg
                        .filter(|_| t.characteristic == Characteristic::Angularity)
                    {
                        out.dimensions.push(PmiDimension {
                            name: "basic angle".into(),
                            angular: true,
                            nominal: angle.to_radians(),
                            features: vec![probes(document, &element), probes(document, &datum)],
                            ..Default::default()
                        });
                    }
                }
                let mut datum_modifiers: Vec<Vec<String>> = refs
                    .iter()
                    .map(|r| {
                        r.modifier
                            .map(|m| m.word().to_string())
                            .into_iter()
                            .collect()
                    })
                    .collect();
                if matches!(
                    t.characteristic,
                    Characteristic::CircularRunout | Characteristic::TotalRunout
                ) && datums.len() == 2
                {
                    datums = vec![datums.join("-")];
                    datum_modifiers = vec![datum_modifiers.into_iter().flatten().collect()];
                }
                let kind = match t.characteristic {
                    Characteristic::Circularity => "roundness".into(),
                    Characteristic::Sphericity => "surface_profile".into(),
                    other => other.key(),
                };
                out.tolerances.push(PmiTolerance {
                    kind,
                    name: t.name,
                    magnitude: t.limit,
                    datums,
                    at: probes(document, &element),
                    modifiers: t
                        .material
                        .map(|m| m.word().to_string())
                        .into_iter()
                        .chain(t.unequal.map(|_| "unequally_disposed".to_string()))
                        .collect(),
                    datum_modifiers,
                });
            }
            crate::dimension::KIND => {
                let Ok(d) = Dimension::from_json(data) else {
                    continue;
                };
                if !scans.contains(&d.scan) {
                    continue;
                }
                let Some(elements) = d
                    .elements
                    .iter()
                    .map(|id| crate::element_at(document, *id))
                    .collect::<Option<Vec<_>>>()
                else {
                    continue;
                };
                let angular = d.kind == DimKind::Angle;
                let value = |v: f64| if angular { v.to_radians() } else { v };
                let band = d.plus != 0.0 || d.minus != 0.0;
                out.dimensions.push(PmiDimension {
                    name: match d.kind {
                        DimKind::Diameter => "diameter",
                        DimKind::Distance => "linear distance",
                        DimKind::Angle => "angle",
                    }
                    .into(),
                    angular,
                    nominal: value(d.nominal),
                    plus: band.then(|| value(d.plus)),
                    minus: band.then(|| -value(d.minus)),
                    features: elements.iter().map(|e| probes(document, e)).collect(),
                });
            }
            _ => {}
        }
    }
    out
}

fn datum_label(mut index: usize) -> String {
    let mut label = Vec::new();
    loop {
        label.push(b'A' + (index % 26) as u8);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    label.into_iter().rev().map(char::from).collect()
}

/// Samples on the measured patch, or on the fitted surface when there is
/// no mesh region (a construction). Probes use the scan's placement, not
/// the reference's; radial features are probed on their walls, not axes.
fn probes(document: &Document, element: &Element) -> Vec<[f64; 3]> {
    let mut points = Vec::new();
    if let Some((mesh, _)) = document.local_geometry(element.scan) {
        let count = vertices(&element.region).count();
        points.extend(
            vertices(&element.region)
                .step_by(count.div_ceil(32).max(1))
                .filter_map(|i| mesh.positions.get(i as usize).map(|p| p.map(f64::from))),
        );
    }
    if points.is_empty() {
        let middle = |[a, b]: [f64; 2]| (a + b) / 2.0;
        let along = match element.result.span {
            Span::Axis(t) => middle(t),
            _ => 0.0,
        };
        let ring = |centre, normal, radius| {
            let (u, v) = basis(normal);
            [u, v, scale(u, -1.0), scale(v, -1.0)]
                .map(|dir| add(centre, scale(dir, radius)))
                .to_vec()
        };
        points = match element.result.shape {
            FittedShape::Plane { origin, normal } => vec![match element.result.span {
                Span::Patch { u, across } => add(
                    origin,
                    add(
                        scale(u, middle(across[0])),
                        scale(cross(normal, u), middle(across[1])),
                    ),
                ),
                _ => origin,
            }],
            FittedShape::Line { point, direction } => vec![add(point, scale(direction, along))],
            FittedShape::Circle {
                centre,
                normal,
                radius,
            } => ring(centre, normal, radius),
            FittedShape::Cylinder {
                point,
                axis,
                radius,
            } => ring(add(point, scale(axis, along)), axis, radius),
            FittedShape::Cone {
                apex,
                axis,
                half_angle,
            } => ring(
                add(apex, scale(axis, along)),
                axis,
                along.abs() * half_angle.tan(),
            ),
            FittedShape::Sphere { centre, radius } => {
                let mut p = ring(centre, [0.0, 0.0, 1.0], radius);
                p.extend([
                    add(centre, [0.0, 0.0, radius]),
                    add(centre, [0.0, 0.0, -radius]),
                ]);
                p
            }
            FittedShape::Torus {
                centre,
                axis,
                major,
                minor,
            } => ring(centre, axis, major + minor),
        };
    }
    let placement = document.body_placement(element.scan);
    let rotation = placement.quat().as_dquat();
    let translation = placement.offset().as_dvec3();
    points
        .into_iter()
        .map(|p| (rotation * glam::DVec3::from_array(p) + translation).to_array())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::Fitted;
    use core_document::{BodyPlacement, Workbench};

    #[test]
    fn reference_pmi_uses_ordered_datums_and_world_probes() {
        let mut doc = Document::new("Inspection");
        let reference = doc.create_body(Some("Reference".into()));
        let scan = doc.create_body(Some("Scan".into()));
        doc.set_imported_geometry(
            scan,
            core_document::ImportedGeometry {
                mesh: std::sync::Arc::new(kernel_api::TriMesh {
                    positions: vec![[1.0, 2.0, 3.0]],
                    ..Default::default()
                }),
                source_asset: None,
                revision: 0,
                bounds_mm: None,
                brep_blob_path: None,
                mesh_path: None,
                face_colors_path: None,
                health: None,
            },
        );
        let place = BodyPlacement::new(
            glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            glam::Vec3::new(20.0, 30.0, 40.0),
        );
        doc.set_body_placement(scan, place);
        doc.set_body_placement(
            reference,
            BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::splat(-100.0)),
        );
        let mut ids = Vec::new();
        for x in [0.0, 5.0, 10.0] {
            let element = Element {
                name: "Plane".into(),
                scan,
                kind: kernel_api::FitKind::Plane,
                cutoff: None,
                method: Default::default(),
                region: if x == 0.0 { vec![[0, 1]] } else { vec![] },
                revision: 0,
                result: Fitted {
                    shape: FittedShape::Plane {
                        origin: [x, 2.0, 3.0],
                        normal: [0.0, 0.0, 1.0],
                    },
                    sigma: 0.0,
                    form: (0.0, 0.0),
                    used: 0,
                    points: 0,
                    span: Span::None,
                    zone: None,
                },
            };
            ids.push(
                doc.add_feature_in_body(element, "Plane".into(), Some(scan))
                    .unwrap(),
            );
        }
        for datum in [ids[2], ids[1], ids[2]] {
            doc.add_feature_in_body(
                Tolerance {
                    name: "Parallel".into(),
                    scan,
                    characteristic: Characteristic::Parallelism,
                    element: ids[0],
                    datum: Some(datum),
                    limit: 0.1,
                    angle_deg: None,
                    ..Default::default()
                },
                "Parallel".into(),
                Some(scan),
            )
            .unwrap();
        }
        for kind in [DimKind::Distance, DimKind::Angle] {
            doc.add_feature_in_body(
                Dimension {
                    name: "Size".into(),
                    scan,
                    kind,
                    elements: ids[..2].to_vec(),
                    nominal: 30.0,
                    plus: 0.2,
                    minus: 0.1,
                },
                "Size".into(),
                Some(scan),
            )
            .unwrap();
        }
        let bench = crate::InspectionWorkbench::default();
        assert!(bench.semantic_pmi(&doc, reference).is_empty());
        for _ in 0..2 {
            let comparison = Comparison::from_json(
                &serde_json::json!({"name": "Comparison", "scan": scan, "reference": reference}),
            )
            .unwrap();
            doc.add_feature_in_body(comparison, "Comparison".into(), Some(scan))
                .unwrap();
        }
        let pmi = bench.semantic_pmi(&doc, reference);
        assert!(bench.semantic_pmi(&doc, scan).is_empty());
        assert_eq!(pmi.tolerances.len(), 3);
        assert_eq!(
            pmi.datums
                .iter()
                .map(|d| d.label.as_str())
                .collect::<Vec<_>>(),
            ["A", "B"]
        );
        assert_eq!(
            pmi.tolerances
                .iter()
                .map(|t| t.datums[0].as_str())
                .collect::<Vec<_>>(),
            ["A", "B", "A"]
        );
        let close = |got: [f64; 3], own| {
            let want = place.point(own).map(f64::from);
            assert!(
                (0..3).all(|i| (got[i] - want[i]).abs() < 1e-5),
                "{got:?} vs {want:?}"
            );
        };
        close(pmi.tolerances[0].at[0], [1.0, 2.0, 3.0]);
        close(pmi.datums[0].at[0], [10.0, 2.0, 3.0]);
        close(pmi.datums[1].at[0], [5.0, 2.0, 3.0]);
        for (dimension, factor) in pmi
            .dimensions
            .iter()
            .zip([1.0, std::f64::consts::PI / 180.0])
        {
            assert!((dimension.nominal - 30.0 * factor).abs() < 1e-12);
            assert!((dimension.plus.unwrap() - 0.2 * factor).abs() < 1e-12);
            assert!((dimension.minus.unwrap() + 0.1 * factor).abs() < 1e-12);
            assert_eq!(dimension.features.len(), 2);
            close(dimension.features[1][0], [5.0, 2.0, 3.0]);
        }
        assert_eq!(datum_label(26), "AA");
    }
}
