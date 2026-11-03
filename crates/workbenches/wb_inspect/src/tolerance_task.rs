//! A tolerance task keeps its opening data for Cancel. Accepted changes
//! record the same arguments the tolerance command consumes.

use crate::tolerance::Tolerance;
use core_document::{FeatureId, WorkbenchFeature};

pub(crate) struct Task {
    pub id: FeatureId,
    pub opening: Option<Tolerance>,
}

pub(crate) fn args(t: &Tolerance, id: Option<FeatureId>) -> core_document::CommandArgs {
    let mut args = t.to_json().as_object().cloned().unwrap_or_default();
    for key in ["name", "scan", "angle_deg"] {
        args.remove(key);
    }
    args.retain(|_, v| !v.is_null());
    if let Some(a) = t.angle_deg {
        args.insert("angle".into(), serde_json::json!(a));
    }
    if let Some(id) = id {
        args.insert("tolerance".into(), serde_json::json!(id));
    }
    args.into_iter().collect()
}

#[cfg(feature = "egui")]
pub(crate) fn draw(ui: &mut egui::Ui, document: &core_document::Document, t: &mut Tolerance) {
    use crate::tolerance::{Characteristic as C, DatumRef, Material};
    use ui_kit::widgets::overline as section_header;
    let choices: Vec<_> = document
        .feature_tree()
        .all_nodes()
        .filter_map(|(id, n)| {
            crate::element_at(document, *id)
                .filter(|e| e.scan == t.scan)
                .map(|_| (*id, n.name.clone()))
        })
        .collect();
    section_header(ui, "Characteristic");
    egui::ComboBox::from_id_salt("characteristic")
        .selected_text(t.characteristic.word())
        .show_ui(ui, |ui| {
            for c in C::ALL {
                ui.selectable_value(&mut t.characteristic, c, c.word());
            }
        });
    ui.horizontal(|ui| {
        ui.label("Limit (mm)");
        ui.add(
            egui::DragValue::new(&mut t.limit)
                .speed(0.01)
                .range(0.0..=f64::MAX),
        );
    });
    let mut element = Some(t.element);
    pick(ui, "Element", &mut element, &choices);
    if let Some(id) = element {
        t.element = id;
    }
    if t.characteristic == C::Angularity {
        optional_number(ui, "Basic angle (°)", &mut t.angle_deg, 90.0);
    }
    section_header(ui, "Datum frame");
    let mut refs = t.references();
    for i in 0..3 {
        let mut id = refs.get(i).map(|r| r.element);
        pick(
            ui,
            ["Primary", "Secondary", "Tertiary"][i],
            &mut id,
            &choices,
        );
        match (refs.get(i), id) {
            (Some(_), None) => {
                refs.truncate(i);
            }
            (Some(_), Some(id)) => refs[i].element = id,
            (None, Some(id)) => refs.push(DatumRef {
                element: id,
                modifier: None,
                size: None,
            }),
            _ => {}
        }
        if let Some(r) = refs.get_mut(i) {
            ui.push_id(i, |ui| {
                material(ui, "Datum modifier", &mut r.modifier);
                if r.modifier.is_some() {
                    optional_number(ui, "Boundary size (mm)", &mut r.size, 10.0);
                }
            });
        }
    }
    if refs != t.references() {
        t.datum = None;
        t.datums = refs;
    }
    if t.characteristic == C::Position {
        section_header(ui, "True position");
        optional_vector(ui, "Basic offsets (mm)", &mut t.basic, [0.0; 3]);
        optional_vector(
            ui,
            "Zone direction",
            &mut t.basic_direction,
            [0.0, 0.0, 1.0],
        );
        material(ui, "Material", &mut t.material);
        if t.material.is_some() {
            optional_number(ui, "Material size (mm)", &mut t.material_size, 10.0);
        }
        let mut face = t.reference_face.map(f64::from);
        optional_number(ui, "Nominal face index", &mut face, 0.0);
        t.reference_face = face.map(|v| v.max(0.0) as u32);
    }
    if t.characteristic.is_profile() || t.characteristic == C::Position {
        let comparisons = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| {
                n.workbench_id.as_str() == crate::compare::KIND && n.body == Some(t.scan)
            })
            .map(|(id, n)| (*id, n.name.clone()))
            .collect::<Vec<_>>();
        pick(ui, "Comparison", &mut t.comparison, &comparisons);
    }
    if t.characteristic.is_profile() {
        optional_number(ui, "Zone centre offset (mm)", &mut t.unequal, 0.0);
    }
    if t.characteristic == C::LineProfile {
        let sections = document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| {
                n.workbench_id.as_str() == crate::section::KIND && n.body == Some(t.scan)
            })
            .map(|(id, n)| (*id, n.name.clone()))
            .collect::<Vec<_>>();
        pick(ui, "Section", &mut t.section, &sections);
    }
    if matches!(t.characteristic, C::Symmetry | C::Position) {
        pick(ui, "Opposed plane (slot)", &mut t.opposed, &choices);
    }
    if t.characteristic == C::CircularRunout {
        optional_number(ui, "Section width (mm)", &mut t.section_width, 0.1);
    }
    fn material(ui: &mut egui::Ui, label: &str, value: &mut Option<Material>) {
        egui::ComboBox::from_id_salt(label)
            .selected_text(format!(
                "{label}: {}",
                value.map_or("None", Material::symbol)
            ))
            .show_ui(ui, |ui| {
                ui.selectable_value(value, None, "None");
                ui.selectable_value(value, Some(Material::Mmc), "MMC Ⓜ");
                ui.selectable_value(value, Some(Material::Lmc), "LMC Ⓛ");
            });
    }
}

#[cfg(feature = "egui")]
fn pick(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Option<FeatureId>,
    choices: &[(FeatureId, String)],
) {
    let name = choices
        .iter()
        .find(|(id, _)| Some(*id) == *value)
        .map_or("None", |(_, n)| n.as_str());
    egui::ComboBox::from_id_salt(label)
        .selected_text(format!("{label}: {name}"))
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, "None");
            for (id, name) in choices {
                ui.selectable_value(value, Some(*id), name);
            }
        });
}
#[cfg(feature = "egui")]
fn optional_number(ui: &mut egui::Ui, label: &str, value: &mut Option<f64>, default: f64) {
    ui.horizontal(|ui| {
        let mut enabled = value.is_some();
        if ui.checkbox(&mut enabled, label).changed() {
            *value = enabled.then_some(default);
        }
        if let Some(v) = value {
            ui.add(egui::DragValue::new(v).speed(0.01));
        }
    });
}
#[cfg(feature = "egui")]
fn optional_vector(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Option<[f64; 3]>,
    default: [f64; 3],
) {
    let mut enabled = value.is_some();
    if ui.checkbox(&mut enabled, label).changed() {
        *value = enabled.then_some(default);
    }
    if let Some(v) = value {
        ui.horizontal(|ui| {
            for x in v {
                ui.add(egui::DragValue::new(x).speed(0.01));
            }
        });
    }
}
