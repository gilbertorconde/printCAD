//! The task panel of a generated sketch: its numbers, each a field a
//! formula may set, what they come to, and the way out to a plain sketch.
//! Every change makes the curves again at once, so the sketch on screen is
//! the preview.

use core_document::{FeatureId, Parameter, WorkbenchFeature, WorkbenchRuntimeContext};
use egui::RichText;
use serde_json::{Map, Value, json};
use ui_kit::tokens::*;
use ui_kit::widgets::{
    Card, FormulaEdit, FormulaField, Note, QtyField, check_row, field_label, mono_label, note_card,
    overline, secondary_button, select_field, small_secondary_button,
};
use ui_kit::{icon, sans_semibold};

use super::{
    Generator, SPROCKET_CHAINS, ShaftAnalysis, ShaftForce, ShaftLoads, ShaftSpec, analyse,
    regenerate, summary,
};
use crate::feature::SketchFeature;

/// The width of the label column.
const LABEL: f32 = 110.0;

/// Draw the panel for generated sketch `id` and apply what it changes.
pub(crate) fn show(ui: &mut egui::Ui, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) {
    let Some(node) = ctx.document.get_feature_meta(id).cloned() else {
        return;
    };
    // As its formulas leave it: what the fields show.
    let values = ctx
        .document
        .feature_values(id)
        .cloned()
        .unwrap_or_else(|| node.data.clone());
    let Some(generator) = SketchFeature::from_json(&values)
        .ok()
        .and_then(|f| f.generator)
    else {
        return;
    };
    let mut data = node.data.clone();
    let mut edited: Vec<String> = Vec::new();
    let mut formulas: Vec<(String, Option<String>)> = Vec::new();

    Card::new().show(ui, |ui| {
        ui.horizontal(|ui| {
            icon::draw(ui, generator.icon(), 16.0, ACCENT);
            ui.label(
                RichText::new(generator.label())
                    .font(sans_semibold(FONT_MD))
                    .color(TEXT1),
            );
        });
        ui.add_space(SPACE_2);
        if let Generator::Sprocket(spec) = &generator {
            let current = spec
                .chain()
                .and_then(|name| SPROCKET_CHAINS.iter().position(|(n, _, _)| *n == name));
            let mut chosen = current.unwrap_or(SPROCKET_CHAINS.len());
            let mut options: Vec<(usize, &str)> = SPROCKET_CHAINS
                .iter()
                .enumerate()
                .map(|(i, (name, _, _))| (i, *name))
                .collect();
            options.push((SPROCKET_CHAINS.len(), "Custom"));
            let picked = ui
                .horizontal(|ui| {
                    label_cell(ui, "Chain");
                    select_field(ui, ("sprocket_chain", id), &mut chosen, &options, 140.0)
                })
                .inner;
            if picked && let Some((_, pitch, roller)) = SPROCKET_CHAINS.get(chosen) {
                set(&mut data, "/generator/Sprocket/pitch", *pitch);
                set(&mut data, "/generator/Sprocket/roller", *roller);
                edited.extend(["pitch".to_string(), "roller".to_string()]);
            }
        }
        if let Generator::Gear(spec) = &generator {
            let mut internal = spec.internal;
            if check_row(ui, &mut internal, "Internal (ring) gear").changed() {
                set_value(&mut data, "/generator/Gear/internal", json!(internal));
                edited.push("internal".into());
            }
            if !spec.internal {
                let mut undercut = spec.undercut;
                if check_row(ui, &mut undercut, "Root as the cutter leaves it")
                    .on_hover_text(
                        "The trochoid a rack cutter's rounded tip traces, which undercuts the \
                         flanks of a pinion of few teeth; off, a root fillet arc",
                    )
                    .changed()
                {
                    set_value(&mut data, "/generator/Gear/undercut", json!(undercut));
                    edited.push("undercut".into());
                }
                keyway_check(ui, &mut data, &mut edited, "Gear", spec.keyway);
            }
        }
        if let Generator::Sprocket(spec) = &generator {
            keyway_check(ui, &mut data, &mut edited, "Sprocket", spec.keyway);
        }
        if let Generator::Keyway(spec) = &generator {
            let mut rounded = spec.rounded;
            if check_row(ui, &mut rounded, "Round ends").changed() {
                set_value(&mut data, "/generator/Keyway/rounded", json!(rounded));
                edited.push("rounded".into());
            }
        }
        let params = generator.parameters();
        for (i, p) in params.iter().enumerate() {
            match (&generator, p.key.as_str()) {
                (Generator::Gear(spec), "bore" | "root_fillet") if spec.internal => continue,
                (Generator::Gear(spec), "rim") if !spec.internal => continue,
                (Generator::Gear(spec), "keyway_width" | "keyway_depth")
                    if spec.internal || !spec.keyway.on =>
                {
                    continue;
                }
                (Generator::Sprocket(spec), "keyway_width" | "keyway_depth") if !spec.keyway.on => {
                    continue;
                }
                _ => {}
            }
            if let Some(section) = shaft_section_start(p) {
                ui.add_space(SPACE_2);
                overline(ui, &format!("Section {section}"));
            } else if i == 0 && matches!(generator, Generator::Shaft(_)) {
                overline(ui, "Ends");
            }
            let value = values.pointer(&p.pointer).and_then(Value::as_f64);
            let Some(value) = value else { continue };
            match number_row(ui, ctx, id, p, value / p.scale) {
                Some(FormulaEdit::Value(v)) => {
                    let stored = v * p.scale;
                    set(
                        &mut data,
                        &p.pointer,
                        if p.integer { stored.round() } else { stored },
                    );
                    if ctx.document.feature_formula(id, &p.key).is_some() {
                        formulas.push((p.key.clone(), None));
                    }
                    edited.push(field_of(&p.pointer));
                }
                Some(FormulaEdit::Formula(text)) => formulas.push((p.key.clone(), Some(text))),
                None => {}
            }
        }
        if let Generator::Shaft(spec) = &generator {
            ui.add_space(SPACE_2);
            ui.horizontal(|ui| {
                if small_secondary_button(ui, "Add section").clicked()
                    && let Some(sections) = data
                        .pointer_mut("/generator/Shaft/sections")
                        .and_then(Value::as_array_mut)
                {
                    let last = sections.last().cloned().unwrap_or(json!({}));
                    sections.push(last);
                    edited.push("sections".into());
                }
                let n = spec.sections.len();
                if n > 1 && small_secondary_button(ui, "Remove last section").clicked() {
                    if let Some(sections) = data
                        .pointer_mut("/generator/Shaft/sections")
                        .and_then(Value::as_array_mut)
                    {
                        sections.pop();
                    }
                    for field in ["length", "diameter", "chamfer", "fillet"] {
                        let key = format!("/generator/Shaft/sections/{}/{field}", n - 1);
                        if ctx.document.feature_formula(id, &key).is_some() {
                            formulas.push((key, None));
                        }
                    }
                    edited.push("sections".into());
                }
            });
        }
    });

    measured(ui, &generator);
    if let Generator::Shaft(spec) = &generator
        && let Some(loads) = loads_card(ui, spec)
    {
        set_value(
            &mut data,
            "/generator/Shaft/loads",
            serde_json::to_value(&loads).unwrap_or_default(),
        );
        edited.push("loads".into());
    }

    ui.add_space(SPACE_2);
    let detach = secondary_button(ui, "Detach into a plain sketch")
        .on_hover_text("Keep the curves and forget the numbers, to edit them by hand")
        .clicked();

    for (key, formula) in formulas {
        if let Err(e) = ctx.document.set_feature_formula(id, key, formula) {
            ctx.log_error(format!("Could not set the formula: {e}"));
        }
    }
    if detach {
        apply(ctx, id, data, true);
        ctx.record(
            "sketch.generator",
            object(json!({"sketch": id.0.to_string(), "detach": true})),
            Value::Null,
        );
        return;
    }
    if !edited.is_empty() && apply(ctx, id, data, false) {
        remember(ui, ctx, id, &edited);
    }
    flush(ui, ctx, id);
}

/// Store `data` as the sketch, its curves made again (or its numbers
/// dropped, `detach`). Answers whether it went in.
fn apply(ctx: &mut WorkbenchRuntimeContext, id: FeatureId, data: Value, detach: bool) -> bool {
    let Ok(mut feature) = SketchFeature::from_json(&data) else {
        ctx.log_error("The generated sketch's numbers do not read back");
        return false;
    };
    if detach {
        feature.generator = None;
    } else if let Err(why) = regenerate(&mut feature) {
        ctx.log_warn(why);
    }
    match ctx.document.update_feature_data(id, feature.to_json()) {
        Ok(()) => {
            ctx.document.mark_feature_dirty(id);
            true
        }
        Err(e) => {
            ctx.log_error(format!("Failed to update the sketch: {e}"));
            false
        }
    }
}

/// One number: its label and a field a formula may set.
fn number_row(
    ui: &mut egui::Ui,
    ctx: &WorkbenchRuntimeContext,
    id: FeatureId,
    p: &Parameter,
    value: f64,
) -> Option<FormulaEdit> {
    let formula = ctx.document.feature_formula(id, &p.key);
    let error = ctx
        .document
        .evaluated_slots(id)
        .iter()
        .find(|s| s.key == p.key)
        .and_then(|s| s.result.as_ref().err())
        .map(String::as_str);
    let host = core_document::DocumentFormulas {
        document: ctx.document,
        dim: p.dim,
    };
    let label = p
        .label
        .rsplit_once(' ')
        .filter(|(_, n)| n.parse::<usize>().is_ok())
        .map_or(p.label.as_str(), |(l, _)| l);
    ui.horizontal(|ui| {
        label_cell(ui, label);
        FormulaField::new(egui::Id::new(("generator_field", id, &p.key)), value, &host)
            .formula(formula)
            .error(error)
            .unit(match p.dim {
                core_document::expr::Dim::LENGTH => "mm",
                core_document::expr::Dim::ANGLE => "°",
                _ => "",
            })
            .decimals(if p.integer { 0 } else { 3 })
            .speed(0.05)
            .show(ui)
    })
    .inner
}

/// What the numbers come to, or why they make nothing.
fn measured(ui: &mut egui::Ui, generator: &Generator) {
    if let Err(why) = generator.outline() {
        note_card(ui, Note::Error, Some("No profile"), &why);
        return;
    }
    let Value::Object(values) = summary(generator) else {
        return;
    };
    Card::new().show(ui, |ui| {
        for (key, value) in values {
            let shown = match (value.as_f64(), value.as_bool()) {
                (Some(v), _) => format!("{v:.3} mm"),
                (_, Some(yes)) => if yes { "yes" } else { "no" }.to_string(),
                _ => continue,
            };
            ui.horizontal(|ui| {
                label_cell(ui, &key.replace('_', " "));
                mono_label(ui, shown, FONT_SM, TEXT1);
            });
        }
    });
}

/// The shaft's loads, each a field, and what they do to it; the loads
/// as edited when a field changed.
fn loads_card(ui: &mut egui::Ui, spec: &ShaftSpec) -> Option<ShaftLoads> {
    let length: f64 = spec.sections.iter().map(|s| s.length).sum();
    let mut loads = spec.loads.clone();
    let mut changed = false;
    ui.add_space(SPACE_2);
    Card::new().show(ui, |ui| {
        overline(ui, "Loads");
        let mut row = |ui: &mut egui::Ui, label: &str, value: &mut f64, unit: &'static str| {
            ui.horizontal(|ui| {
                label_cell(ui, label);
                changed |= QtyField::new(value).unit(unit).speed(0.5).show(ui);
            });
        };
        let [mut first, mut second] = loads.bearings;
        row(ui, "Bearing 1 at", &mut first, "mm");
        row(ui, "Bearing 2 at", &mut second, "mm");
        loads.bearings = [first, second];
        row(ui, "Torque", &mut loads.torque, "N·m");
        if loads.torque != 0.0 {
            row(ui, "Torque from", &mut loads.torque_from, "mm");
            row(ui, "Torque to", &mut loads.torque_to, "mm");
        }
        row(ui, "Modulus", &mut loads.modulus, "GPa");
        let mut remove = None;
        for (i, force) in loads.forces.iter_mut().enumerate() {
            ui.add_space(SPACE_1);
            ui.horizontal(|ui| {
                overline(ui, &format!("Force {}", i + 1));
                if small_secondary_button(ui, "Remove").clicked() {
                    remove = Some(i);
                }
            });
            row(ui, "At", &mut force.at, "mm");
            row(ui, "Force", &mut force.force, "N");
            row(ui, "Angle", &mut force.angle_deg, "°");
        }
        if let Some(i) = remove {
            loads.forces.remove(i);
            changed = true;
        }
        ui.add_space(SPACE_1);
        if small_secondary_button(ui, "Add force").clicked() {
            loads.forces.push(ShaftForce {
                at: length / 2.0,
                ..ShaftForce::default()
            });
            changed = true;
        }
        // Bearings standing together take the shaft's two ends.
        if changed && loads.bearings[0] == loads.bearings[1] {
            loads.bearings = [0.0, length];
        }
        if changed && loads.torque != 0.0 && loads.torque_from == loads.torque_to {
            (loads.torque_from, loads.torque_to) = (0.0, length);
        }
    });
    if loads.any() {
        match analyse(spec, &loads) {
            Ok(result) => analysis_card(ui, &result),
            Err(why) => {
                note_card(ui, Note::Error, Some("No analysis"), &why);
            }
        }
    }
    changed.then_some(loads)
}

/// Reactions, the worst moment, stress and deflection, and a plot of the
/// stress and deflection along the shaft.
fn analysis_card(ui: &mut egui::Ui, result: &ShaftAnalysis) {
    ui.add_space(SPACE_2);
    Card::new().show(ui, |ui| {
        let line = |ui: &mut egui::Ui, label: &str, text: String| {
            ui.horizontal(|ui| {
                label_cell(ui, label);
                mono_label(ui, text, FONT_SM, TEXT1);
            });
        };
        line(ui, "Bearing 1", format!("{:.1} N", result.reactions[0]));
        line(ui, "Bearing 2", format!("{:.1} N", result.reactions[1]));
        let (m, m_at) = result.max_moment();
        line(
            ui,
            "Max moment",
            format!("{:.2} N·m at {m_at:.1} mm", m / 1e3),
        );
        let (s, s_at) = result.max_stress();
        line(ui, "Max stress", format!("{s:.1} MPa at {s_at:.1} mm"));
        let (d, d_at) = result.max_deflection();
        line(ui, "Max deflection", format!("{d:.4} mm at {d_at:.1} mm"));

        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::new(width, 72.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, BG1);
        let end = result.stations.last().map_or(1.0, |s| s.at).max(1e-9);
        let curve = |of: &dyn Fn(&super::loads::Station) -> f64, peak: f64, color| {
            if peak <= 0.0 {
                return;
            }
            let points: Vec<egui::Pos2> = result
                .stations
                .iter()
                .map(|st| {
                    egui::pos2(
                        rect.left() + rect.width() * (st.at / end) as f32,
                        rect.bottom() - 4.0 - (rect.height() - 8.0) * (of(st) / peak) as f32,
                    )
                })
                .collect();
            painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, color)));
        };
        curve(&|st| st.stress, s, WARNING);
        curve(&|st| st.deflection, d, ACCENT);
        ui.horizontal(|ui| {
            mono_label(ui, "stress", FONT_SM, WARNING);
            mono_label(ui, "deflection", FONT_SM, ACCENT);
        });
    });
}

fn label_cell(ui: &mut egui::Ui, text: &str) {
    let mut chars = text.chars();
    let text: String = chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    ui.allocate_ui(egui::Vec2::new(LABEL, INPUT), |ui| {
        ui.set_min_width(LABEL);
        field_label(ui, &text);
    });
}

/// The section a shaft's parameter opens, counted from one, when it is the
/// first of that section's.
fn shaft_section_start(p: &Parameter) -> Option<usize> {
    let rest = p.pointer.strip_prefix("/generator/Shaft/sections/")?;
    let (index, field) = rest.split_once('/')?;
    (field == "length").then(|| index.parse::<usize>().ok().map(|i| i + 1))?
}

/// The generator's field a pointer lands in: `teeth`, or `sections` for
/// anything inside a shaft's sections.
/// The check that cuts a keyway into the bore; its width and depth, 0
/// for the standard key's by the bore, are fields below it.
fn keyway_check(
    ui: &mut egui::Ui,
    data: &mut Value,
    edited: &mut Vec<String>,
    variant: &str,
    keyway: super::BoreKeyway,
) {
    let mut on = keyway.on;
    if check_row(ui, &mut on, "Keyway in the bore")
        .on_hover_text("Its width and depth at 0 take the standard key's (DIN 6885) by the bore")
        .changed()
    {
        set_value(
            data,
            &format!("/generator/{variant}/keyway"),
            serde_json::to_value(super::BoreKeyway { on, ..keyway }).unwrap_or_default(),
        );
        edited.push("keyway".into());
    }
}

fn field_of(pointer: &str) -> String {
    pointer.split('/').nth(3).unwrap_or_default().to_string()
}

fn set(data: &mut Value, pointer: &str, value: f64) {
    if let Some(slot) = data.pointer_mut(pointer) {
        *slot = json!(value);
    }
}

/// Set the field at `pointer`, adding it to its object when the stored
/// data has none.
fn set_value(data: &mut Value, pointer: &str, value: Value) {
    let Some((parent, field)) = pointer.rsplit_once('/') else {
        return;
    };
    if let Some(object) = data.pointer_mut(parent).and_then(Value::as_object_mut) {
        object.insert(field.to_string(), value);
    }
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// Where the fields changed and not yet recorded are kept while a drag
/// goes on, so the recording takes where it ended rather than every step
/// on the way.
fn pending_id(id: FeatureId) -> egui::Id {
    egui::Id::new(("generator_record", id))
}

fn remember(ui: &egui::Ui, ctx: &WorkbenchRuntimeContext, id: FeatureId, fields: &[String]) {
    let Some(generator) = ctx
        .document
        .get_feature_data(id)
        .and_then(|d| d.get("generator"))
        .and_then(Value::as_object)
        .and_then(|o| o.values().next())
        .cloned()
    else {
        return;
    };
    let mut pending: Map<String, Value> = ui
        .data(|d| d.get_temp::<Map<String, Value>>(pending_id(id)))
        .unwrap_or_default();
    for field in fields {
        if let Some(v) = generator.get(field) {
            pending.insert(field.clone(), v.clone());
        }
    }
    ui.data_mut(|d| d.insert_temp(pending_id(id), pending));
}

/// Record what changed once the pointer is let go.
fn flush(ui: &egui::Ui, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) {
    if ui.input(|i| i.pointer.any_down()) {
        return;
    }
    let Some(mut pending) = ui.data_mut(|d| d.remove_temp::<Map<String, Value>>(pending_id(id)))
    else {
        return;
    };
    if pending.is_empty() {
        return;
    }
    pending.insert("sketch".into(), json!(id.0.to_string()));
    let answer = ctx
        .document
        .feature_values(id)
        .and_then(|d| SketchFeature::from_json(d).ok())
        .and_then(|f| f.generator)
        .map(|g| summary(&g))
        .unwrap_or(Value::Null);
    ctx.record("sketch.generator", pending, answer);
}
