//! The task panel of a generated sketch: its numbers, each a field a
//! formula may set, what they come to, and the way out to a plain sketch.
//! Every change makes the curves again at once, so the sketch on screen is
//! the preview. Declared as widgets and drawn by `core_document::panel`.

use bench_api::panel;
use bench_api::{
    ButtonStyle, Caption, Card, Check, Dim, GapSize, NoteKind, PanelEvent, Plot, PlotSeries,
    Quantity, Select, Tone, Widget,
};
use core_document::{FeatureId, Parameter, WorkbenchFeature, WorkbenchRuntimeContext};
use serde_json::{Map, Value, json};

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
    let widgets = widgets(id, &generator, &values);
    let out = core_document::panel::show(
        ui,
        egui::Id::new(("sketch_generator", id)),
        &widgets,
        Some(&*ctx.document),
    );
    let mut data = node.data.clone();
    let mut edited: Vec<String> = Vec::new();
    // A number a formula now sets takes its value from the formula.
    let by_formula: Vec<String> = out
        .formulas
        .iter()
        .filter(|set| set.formula.is_some())
        .map(|set| set.id.clone())
        .collect();
    let mut formulas: Vec<(String, Option<String>)> = out
        .formulas
        .into_iter()
        .map(|set| (set.bind.key, set.formula))
        .collect();
    let mut detach = false;
    let (mut loads, length) = match &generator {
        Generator::Shaft(spec) => (
            Some(spec.loads.clone()),
            spec.sections.iter().map(|s| s.length).sum(),
        ),
        _ => (None, 0.0),
    };
    let mut loads_changed = false;
    for event in &out.events {
        if by_formula.iter().any(|f| f == event.id()) {
            continue;
        }
        match event {
            PanelEvent::Button { id: what } if what == "detach" => detach = true,
            _ => {
                if loads
                    .as_mut()
                    .is_some_and(|l| loads_event(l, length, event))
                {
                    loads_changed = true;
                } else {
                    let to = (&mut data, &mut edited, &mut formulas);
                    edit(ctx, id, &generator, to, event);
                }
            }
        }
    }
    if let Some(mut loads) = loads
        && loads_changed
    {
        // Bearings standing together take the shaft's two ends.
        if loads.bearings[0] == loads.bearings[1] {
            loads.bearings = [0.0, length];
        }
        if loads.torque != 0.0 && loads.torque_from == loads.torque_to {
            (loads.torque_from, loads.torque_to) = (0.0, length);
        }
        set_value(
            &mut data,
            "/generator/Shaft/loads",
            serde_json::to_value(&loads).unwrap_or_default(),
        );
        edited.push("loads".into());
    }

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

/// The id of parameter `key`'s field.
fn param_field(key: &str) -> String {
    format!("param:{key}")
}

/// The generator's numbers and switches as a card, what they come to,
/// a shaft's loads and what they do to it, and the way out.
fn widgets(id: FeatureId, generator: &Generator, values: &Value) -> Vec<Widget> {
    let mut rows: Vec<Widget> = Vec::new();
    if let Generator::Sprocket(spec) = generator {
        let current = spec
            .chain()
            .and_then(|name| SPROCKET_CHAINS.iter().position(|(n, _, _)| *n == name));
        let mut options: Vec<String> = SPROCKET_CHAINS
            .iter()
            .map(|(name, _, _)| name.to_string())
            .collect();
        options.push("Custom".into());
        rows.push(
            Select {
                id: "chain".into(),
                label: "Chain".into(),
                options,
                selected: current.unwrap_or(SPROCKET_CHAINS.len()),
                width: Some(140.0),
                ..Default::default()
            }
            .into(),
        );
    }
    if let Generator::Gear(spec) = generator {
        rows.push(Widget::from(Check {
            hint: "".into(),
            ..panel::check("internal", "", "Internal (ring) gear", spec.internal)
        }));
        if !spec.internal {
            rows.push(Widget::from(Check { hint: "The trochoid a rack cutter's rounded tip traces, which undercuts the flanks of a \
                 pinion of few teeth; off, a root fillet arc".into(), ..panel::check("undercut", "", "Root as the cutter leaves it", spec.undercut) }));
            rows.push(keyway_check(spec.keyway));
        }
    }
    if let Generator::Sprocket(spec) = generator {
        rows.push(keyway_check(spec.keyway));
    }
    if let Generator::Keyway(spec) = generator {
        rows.push(Widget::from(Check {
            hint: "".into(),
            ..panel::check("rounded", "", "Round ends", spec.rounded)
        }));
    }
    for (i, p) in generator.parameters().iter().enumerate() {
        match (generator, p.key.as_str()) {
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
            rows.push(panel::gap(GapSize::Medium));
            rows.push(panel::overline(&format!("Section {section}")));
        } else if i == 0 && matches!(generator, Generator::Shaft(_)) {
            rows.push(panel::overline("Ends"));
        }
        let Some(value) = values.pointer(&p.pointer).and_then(Value::as_f64) else {
            continue;
        };
        rows.push(number(id, p, value / p.scale));
    }
    if let Generator::Shaft(spec) = generator {
        rows.push(panel::gap(GapSize::Medium));
        let mut buttons = vec![panel::button(
            "shaft.add",
            "Add section",
            ButtonStyle::Small,
            true,
        )];
        if spec.sections.len() > 1 {
            buttons.push(panel::button(
                "shaft.remove",
                "Remove last section",
                ButtonStyle::Small,
                true,
            ));
        }
        rows.push(panel::flow_no_wrap(buttons));
    }
    let mut widgets = vec![
        Card {
            icon: Some(generator.icon().into()),
            title: generator.label().into(),
            icon_size: 16.0,
            children: vec![panel::gap(GapSize::Medium), panel::fields_at(LABEL, rows)],
            ..Default::default()
        }
        .into(),
    ];
    widgets.extend(measured(generator));
    if let Generator::Shaft(spec) = generator {
        widgets.extend(loads_widgets(spec));
    }
    widgets.push(panel::gap(GapSize::Medium));
    widgets.push(Widget::Hinted {
        hint: "Keep the curves and forget the numbers, to edit them by hand".into(),
        widget: Box::new(Widget::Button {
            id: "detach".into(),
            label: "Detach into a plain sketch".into(),
            style: ButtonStyle::Secondary,
            enabled: true,
        }),
    });
    widgets
}

/// One number: its label and a field a formula may set.
fn number(id: FeatureId, p: &Parameter, value: f64) -> Widget {
    let label = p
        .label
        .rsplit_once(' ')
        .filter(|(_, n)| n.parse::<usize>().is_ok())
        .map_or(p.label.as_str(), |(l, _)| l);
    let dim = match p.dim {
        core_document::expr::Dim::LENGTH => Dim::Length,
        core_document::expr::Dim::ANGLE => Dim::Angle,
        _ => Dim::Number,
    };
    Quantity {
        id: param_field(&p.key),
        label: capitalized(label),
        value,
        dim,
        bind: Some(bench_api::Bind {
            feature: id.0.to_string(),
            key: p.key.clone(),
        }),
        decimals: if p.integer { 0 } else { 3 },
        speed: Some(0.05),
        ..Default::default()
    }
    .into()
}

/// What the numbers come to, or why they make nothing.
fn measured(generator: &Generator) -> Vec<Widget> {
    if let Err(why) = generator.outline() {
        return vec![Widget::Note {
            kind: NoteKind::Error,
            title: Some("No profile".into()),
            text: why,
        }];
    }
    let Value::Object(values) = summary(generator) else {
        return Vec::new();
    };
    let rows: Vec<Widget> = values
        .into_iter()
        .filter_map(|(key, value)| {
            let shown = match (value.as_f64(), value.as_bool()) {
                (Some(v), _) => format!("{v:.3} mm"),
                (_, Some(yes)) => if yes { "yes" } else { "no" }.to_string(),
                _ => return None,
            };
            Some(line(&key.replace('_', " "), shown))
        })
        .collect();
    vec![panel::card(None, "", vec![panel::fields_at(LABEL, rows)])]
}

/// The shaft's loads, each a field, and what they do to it.
fn loads_widgets(spec: &ShaftSpec) -> Vec<Widget> {
    let loads = &spec.loads;
    let load = |id: &str, label: &str, value: f64, unit: &str| -> Widget {
        Quantity {
            id: format!("loads.{id}"),
            label: label.into(),
            value,
            dim: Dim::Number,
            unit: Some(unit.into()),
            speed: Some(0.5),
            ..Default::default()
        }
        .into()
    };
    let mut rows = vec![
        panel::overline("Loads"),
        load("bearing1", "Bearing 1 at", loads.bearings[0], "mm"),
        load("bearing2", "Bearing 2 at", loads.bearings[1], "mm"),
        load("torque", "Torque", loads.torque, "N·m"),
    ];
    if loads.torque != 0.0 {
        rows.push(load("torque_from", "Torque from", loads.torque_from, "mm"));
        rows.push(load("torque_to", "Torque to", loads.torque_to, "mm"));
    }
    rows.push(load("modulus", "Modulus", loads.modulus, "GPa"));
    for (i, force) in loads.forces.iter().enumerate() {
        rows.push(panel::gap(GapSize::Small));
        rows.push(panel::flow_no_wrap(vec![
            panel::overline(&format!("Force {}", i + 1)),
            panel::button(
                &format!("loads.force:{i}.remove"),
                "Remove",
                ButtonStyle::Small,
                true,
            ),
        ]));
        rows.push(load(&format!("force:{i}.at"), "At", force.at, "mm"));
        rows.push(load(&format!("force:{i}.force"), "Force", force.force, "N"));
        rows.push(load(
            &format!("force:{i}.angle"),
            "Angle",
            force.angle_deg,
            "°",
        ));
    }
    rows.push(panel::gap(GapSize::Small));
    rows.push(panel::button(
        "loads.add_force",
        "Add force",
        ButtonStyle::Small,
        true,
    ));
    let mut widgets = vec![
        panel::gap(GapSize::Medium),
        panel::card(None, "", vec![panel::fields_at(LABEL, rows)]),
    ];
    if loads.any() {
        match analyse(spec, loads) {
            Ok(result) => widgets.extend(analysis_widgets(&result)),
            Err(why) => widgets.push(Widget::Note {
                kind: NoteKind::Error,
                title: Some("No analysis".into()),
                text: why,
            }),
        }
    }
    widgets
}

/// A change to the shaft's loads; whether `event` was one.
fn loads_event(loads: &mut ShaftLoads, length: f64, event: &PanelEvent) -> bool {
    match event {
        PanelEvent::Number { id, value } => {
            let Some(field) = id.strip_prefix("loads.") else {
                return false;
            };
            let slot = match field {
                "bearing1" => &mut loads.bearings[0],
                "bearing2" => &mut loads.bearings[1],
                "torque" => &mut loads.torque,
                "torque_from" => &mut loads.torque_from,
                "torque_to" => &mut loads.torque_to,
                "modulus" => &mut loads.modulus,
                _ => {
                    let Some((i, what)) = field
                        .strip_prefix("force:")
                        .and_then(|r| r.split_once('.'))
                        .and_then(|(i, w)| Some((i.parse::<usize>().ok()?, w)))
                    else {
                        return false;
                    };
                    let Some(force) = loads.forces.get_mut(i) else {
                        return false;
                    };
                    match what {
                        "at" => &mut force.at,
                        "force" => &mut force.force,
                        "angle" => &mut force.angle_deg,
                        _ => return false,
                    }
                }
            };
            *slot = *value;
            true
        }
        PanelEvent::Button { id } if id == "loads.add_force" => {
            loads.forces.push(ShaftForce {
                at: length / 2.0,
                ..ShaftForce::default()
            });
            true
        }
        PanelEvent::Button { id } => {
            let Some(i) = id
                .strip_prefix("loads.force:")
                .and_then(|r| r.strip_suffix(".remove"))
                .and_then(|i| i.parse::<usize>().ok())
            else {
                return false;
            };
            if i < loads.forces.len() {
                loads.forces.remove(i);
            }
            true
        }
        _ => false,
    }
}

/// Where an edit goes: the sketch's data, the fields it changed, and the
/// formulas to set or take away.
type Edits<'a> = (
    &'a mut Value,
    &'a mut Vec<String>,
    &'a mut Vec<(String, Option<String>)>,
);

/// A change to the generator's numbers or switches, into `data`.
fn edit(
    ctx: &WorkbenchRuntimeContext,
    id: FeatureId,
    generator: &Generator,
    (data, edited, formulas): Edits,
    event: &PanelEvent,
) {
    match event {
        PanelEvent::Choice { id: what, index } if what == "chain" => {
            if let Some((_, pitch, roller)) = SPROCKET_CHAINS.get(*index) {
                set(data, "/generator/Sprocket/pitch", *pitch);
                set(data, "/generator/Sprocket/roller", *roller);
                edited.extend(["pitch".to_string(), "roller".to_string()]);
            }
        }
        PanelEvent::Toggle { id: what, on } => {
            let (variant, field) = match (generator, what.as_str()) {
                (Generator::Gear(_), "internal" | "undercut") => ("Gear", what.as_str()),
                (Generator::Keyway(_), "rounded") => ("Keyway", "rounded"),
                (Generator::Gear(spec), "keyway") => {
                    keyway(data, "Gear", spec.keyway, *on);
                    edited.push("keyway".into());
                    return;
                }
                (Generator::Sprocket(spec), "keyway") => {
                    keyway(data, "Sprocket", spec.keyway, *on);
                    edited.push("keyway".into());
                    return;
                }
                _ => return,
            };
            set_value(data, &format!("/generator/{variant}/{field}"), json!(on));
            edited.push(field.into());
        }
        PanelEvent::Number { id: what, value } => {
            let Some(key) = what.strip_prefix("param:") else {
                return;
            };
            let params = generator.parameters();
            let Some(p) = params.iter().find(|p| p.key == key) else {
                return;
            };
            let stored = value * p.scale;
            set(
                data,
                &p.pointer,
                if p.integer { stored.round() } else { stored },
            );
            edited.push(field_of(&p.pointer));
        }
        PanelEvent::Button { id: what } => match (generator, what.as_str()) {
            (Generator::Shaft(_), "shaft.add") => {
                if let Some(sections) = data
                    .pointer_mut("/generator/Shaft/sections")
                    .and_then(Value::as_array_mut)
                {
                    let last = sections.last().cloned().unwrap_or(json!({}));
                    sections.push(last);
                    edited.push("sections".into());
                }
            }
            (Generator::Shaft(spec), "shaft.remove") => {
                let n = spec.sections.len();
                if let Some(sections) = data
                    .pointer_mut("/generator/Shaft/sections")
                    .and_then(Value::as_array_mut)
                {
                    sections.pop();
                }
                // The removed section's formulas go with it.
                for field in ["length", "diameter", "chamfer", "fillet"] {
                    let key = format!("/generator/Shaft/sections/{}/{field}", n - 1);
                    if ctx.document.feature_formula(id, &key).is_some() {
                        formulas.push((key, None));
                    }
                }
                edited.push("sections".into());
            }
            _ => {}
        },
        _ => {}
    }
}

/// Reactions, the worst moment, stress and deflection, and a plot of the
/// stress and deflection along the shaft.
fn analysis_widgets(result: &ShaftAnalysis) -> Vec<Widget> {
    let (m, m_at) = result.max_moment();
    let (s, s_at) = result.max_stress();
    let (d, d_at) = result.max_deflection();
    let rows = vec![
        line("Bearing 1", format!("{:.1} N", result.reactions[0])),
        line("Bearing 2", format!("{:.1} N", result.reactions[1])),
        line("Max moment", format!("{:.2} N·m at {m_at:.1} mm", m / 1e3)),
        line("Max stress", format!("{s:.1} MPa at {s_at:.1} mm")),
        line("Max deflection", format!("{d:.4} mm at {d_at:.1} mm")),
    ];
    let series = |of: fn(&super::loads::Station) -> f64, tone| PlotSeries {
        points: result.stations.iter().map(|st| [st.at, of(st)]).collect(),
        tone,
    };
    vec![
        panel::gap(GapSize::Medium),
        panel::card(
            None,
            "",
            vec![
                panel::fields_at(LABEL, rows),
                Plot {
                    height: 72.0,
                    series: vec![
                        series(|st| st.stress, Tone::Warning),
                        series(|st| st.deflection, Tone::Accent),
                    ],
                }
                .into(),
                panel::flow_no_wrap(vec![
                    legend("stress", Tone::Warning),
                    legend("deflection", Tone::Accent),
                ]),
            ],
        ),
    ]
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

/// The section a shaft's parameter opens, counted from one, when it is the
/// first of that section's.
fn shaft_section_start(p: &Parameter) -> Option<usize> {
    let rest = p.pointer.strip_prefix("/generator/Shaft/sections/")?;
    let (index, field) = rest.split_once('/')?;
    (field == "length").then(|| index.parse::<usize>().ok().map(|i| i + 1))?
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// A label and what it comes to, in the mono face.
fn line(label: &str, value: String) -> Widget {
    Widget::Value {
        label: capitalized(label),
        value,
        mono: true,
    }
}

fn legend(text: &str, tone: Tone) -> Widget {
    Caption {
        text: text.into(),
        tone,
        mono: true,
        ..Default::default()
    }
    .into()
}

/// The check that cuts a keyway into the bore; its width and depth, 0
/// for the standard key's by the bore, are fields below it.
fn keyway_check(keyway: super::BoreKeyway) -> Widget {
    Widget::from(Check {
        hint: "Its width and depth at 0 take the standard key's (DIN 6885) by the bore".into(),
        ..panel::check("keyway", "", "Keyway in the bore", keyway.on)
    })
}

/// The keyway switched on or off in `variant`'s numbers.
fn keyway(data: &mut Value, variant: &str, keyway: super::BoreKeyway, on: bool) {
    set_value(
        data,
        &format!("/generator/{variant}/keyway"),
        serde_json::to_value(super::BoreKeyway { on, ..keyway }).unwrap_or_default(),
    );
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
