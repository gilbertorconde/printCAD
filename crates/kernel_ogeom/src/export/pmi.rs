//! Face-associated semantic PMI, in the part frame the STEP writer uses.

use std::collections::{HashMap, HashSet};

use ogeom::doc::{Datum, DatumReference, Dimension, Document, GeometricTolerance, MeasureKind};
use ogeom::math::Point;
use ogeom::topo::Shape;

use super::{ExportBody, ExportPmi, face_at};

/// Write `semantic` into `document`, each annotation tied to the faces its
/// probes land on. An annotation none of whose probes lands on a face is
/// left out, and so is a tolerance whose datum was; their names come back.
pub(super) fn write(
    document: &mut Document,
    bodies: &[ExportBody<'_>],
    semantic: &[ExportPmi],
    part_of: &[Option<usize>],
    parts: &[Vec<Shape>],
    single: bool,
) -> Vec<String> {
    let mut left_out = Vec::new();
    let mut used_labels = HashSet::new();
    for pmi in semantic {
        let (Some(body), Some(Some(part))) = (bodies.get(pmi.body), part_of.get(pmi.body)) else {
            continue;
        };
        let mut items = |at: &[[f64; 3]]| -> Option<Vec<_>> {
            let mut faces = Vec::new();
            for p in at {
                let p = match body.transform.filter(|_| single) {
                    Some(m) => std::array::from_fn(|r| {
                        m[r][0] * p[0] + m[r][1] * p[1] + m[r][2] * p[2] + m[r][3]
                    }),
                    None => *p,
                };
                if let Some(face) = face_at(
                    document.model_mut(),
                    &parts[*part],
                    Point::new(p[0], p[1], p[2]),
                ) && !faces.contains(&face)
                {
                    faces.push(face);
                }
            }
            (!faces.is_empty()).then_some(faces)
        };
        let mut labels = HashMap::new();
        let mut datums = Vec::new();
        for datum in &pmi.annotations.datums {
            let Some(faces) = items(&datum.at) else {
                left_out.push(format!("{}: datum {}", body.name, datum.label));
                continue;
            };
            let mut label = datum.label.clone();
            let mut suffix = 2;
            while !used_labels.insert(label.clone()) {
                label = format!("{}{}", datum.label, suffix);
                suffix += 1;
            }
            labels.insert(datum.label.clone(), label.clone());
            datums.push(Datum {
                label,
                items: faces,
            });
        }
        let mut tolerances = Vec::new();
        for t in &pmi.annotations.tolerances {
            let references: Option<Vec<_>> = t
                .datums
                .iter()
                .enumerate()
                .map(|(index, label)| {
                    label
                        .split('-')
                        .map(|part| labels.get(part).cloned())
                        .collect::<Option<Vec<_>>>()
                        .map(|parts| DatumReference {
                            label: parts.join("-"),
                            modifiers: t
                                .datum_modifiers
                                .get(index)
                                .into_iter()
                                .flatten()
                                .cloned()
                                .map(ogeom::doc::DatumModifier::Simple)
                                .collect(),
                        })
                })
                .collect();
            let (Some(references), Some(faces)) = (references, items(&t.at)) else {
                left_out.push(format!("{}: {}", body.name, named(&t.name, &t.kind)));
                continue;
            };
            tolerances.push(GeometricTolerance {
                kind: t.kind.clone(),
                name: t.name.clone(),
                magnitude: t.magnitude,
                modifiers: t.modifiers.clone(),
                datum_system: !references.is_empty(),
                datums: references,
                items: faces,
            });
        }
        let mut dimensions = Vec::new();
        for d in &pmi.annotations.dimensions {
            let features: Option<Vec<_>> = (1..=2)
                .contains(&d.features.len())
                .then(|| d.features.iter().map(|at| items(at)).collect())
                .flatten();
            let Some(features) = features else {
                left_out.push(format!("{}: {}", body.name, d.name));
                continue;
            };
            dimensions.push(Dimension {
                name: d.name.clone(),
                values: vec![d.nominal],
                kind: if d.angular {
                    MeasureKind::Angle
                } else {
                    MeasureKind::Length
                },
                plus: d.plus,
                minus: d.minus,
                location: features.len() == 2,
                features,
            });
        }
        let pmi = document.pmi_mut();
        pmi.datums.extend(datums);
        pmi.tolerances.extend(tolerances);
        pmi.dimensions.extend(dimensions);
    }
    left_out
}

fn named(name: &str, kind: &str) -> String {
    if name.is_empty() {
        kind.replace('_', " ")
    } else {
        name.to_string()
    }
}
