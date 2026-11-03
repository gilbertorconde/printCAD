//! Face-associated semantic PMI, in the part frame the STEP writer uses.

use std::collections::{HashMap, HashSet};

use kernel_api::{KernelError, KernelResult};
use ogeom::doc::{Datum, DatumReference, Dimension, Document, GeometricTolerance, MeasureKind};
use ogeom::math::Point;
use ogeom::topo::Shape;

use super::{ExportBody, ExportPmi, face_at};

pub(super) fn write(
    document: &mut Document,
    bodies: &[ExportBody<'_>],
    semantic: &[ExportPmi],
    part_of: &[Option<usize>],
    parts: &[Vec<Shape>],
    single: bool,
) -> KernelResult<()> {
    let mut used_labels = HashSet::new();
    for pmi in semantic {
        let (Some(body), Some(Some(part))) = (bodies.get(pmi.body), part_of.get(pmi.body)) else {
            continue;
        };
        let mut labels = HashMap::new();
        for datum in &pmi.annotations.datums {
            let mut label = datum.label.clone();
            let mut suffix = 2;
            while !used_labels.insert(label.clone()) {
                label = format!("{}{}", datum.label, suffix);
                suffix += 1;
            }
            labels.insert(datum.label.clone(), label);
        }
        let mut items = |at: &[[f64; 3]]| -> KernelResult<Vec<_>> {
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
            if faces.is_empty() {
                return Err(KernelError::InvalidInput(format!(
                    "{}: a semantic annotation has no matching face",
                    body.name
                )));
            }
            Ok(faces)
        };
        let datums = pmi
            .annotations
            .datums
            .iter()
            .map(|datum| {
                Ok(Datum {
                    label: labels[&datum.label].clone(),
                    items: items(&datum.at)?,
                })
            })
            .collect::<KernelResult<Vec<_>>>()?;
        let tolerances = pmi
            .annotations
            .tolerances
            .iter()
            .map(|t| {
                Ok(GeometricTolerance {
                    kind: t.kind.clone(),
                    name: t.name.clone(),
                    magnitude: t.magnitude,
                    datums: t
                        .datums
                        .iter()
                        .map(|label| {
                            labels
                                .get(label)
                                .cloned()
                                .map(DatumReference::new)
                                .ok_or_else(|| {
                                    KernelError::InvalidInput(format!(
                                        "{}: unknown datum {label}",
                                        body.name
                                    ))
                                })
                        })
                        .collect::<KernelResult<_>>()?,
                    items: items(&t.at)?,
                    ..Default::default()
                })
            })
            .collect::<KernelResult<Vec<_>>>()?;
        let dimensions = pmi
            .annotations
            .dimensions
            .iter()
            .map(|d| {
                if !(1..=2).contains(&d.features.len()) {
                    return Err(KernelError::InvalidInput(format!(
                        "{}: a dimension needs one or two features",
                        body.name
                    )));
                }
                Ok(Dimension {
                    name: d.name.clone(),
                    values: vec![d.nominal],
                    kind: if d.angular {
                        MeasureKind::Angle
                    } else {
                        MeasureKind::Length
                    },
                    plus: d.plus,
                    minus: d.minus,
                    location: d.features.len() == 2,
                    features: d
                        .features
                        .iter()
                        .map(|at| items(at))
                        .collect::<KernelResult<_>>()?,
                })
            })
            .collect::<KernelResult<Vec<_>>>()?;
        let pmi = document.pmi_mut();
        pmi.datums.extend(datums);
        pmi.tolerances.extend(tolerances);
        pmi.dimensions.extend(dimensions);
    }
    Ok(())
}
