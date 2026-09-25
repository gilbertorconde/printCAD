//! Part Design's commands: add a feature, or change one, by numbers, for
//! scripts and other callers that are not a click.
//!
//! A feature is made the way its toolbar button makes it, from the sketch
//! and body the call names instead of the selection, with the defaults the
//! button gives; any field of the feature named in the call replaces its
//! default. `pc.doc.feature{id = ...}` shows a feature's fields.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, FeatureId, ParamKind,
    WorkbenchContext, WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::{Map, Value, json};

use core_document::{AttachmentOffset, BasePlane, DatumAttachment, DatumFeature, DatumShape};

use crate::PartDesignWorkbench;
use crate::feature::PartFeature;

/// The features a command makes, by tool id, and what each is.
const FEATURES: &[(&str, &str)] = &[
    ("part.pad", "Pad a sketch"),
    ("part.pocket", "Cut a sketch into the body"),
    ("part.revolve", "Turn a sketch about an axis"),
    ("part.groove", "Cut a sketch turned about an axis"),
    ("part.loft", "Loft through sketches"),
    ("part.subtractive_loft", "Cut a loft through sketches"),
    ("part.pipe", "Sweep a sketch along a path"),
    ("part.subtractive_pipe", "Cut a sketch swept along a path"),
    ("part.helix", "Sweep a sketch along a helix"),
    ("part.subtractive_helix", "Cut a sketch swept along a helix"),
    (
        "part.primitive",
        "Add a box, cylinder, sphere, cone, torus or wedge",
    ),
    (
        "part.subtractive_primitive",
        "Cut a box, cylinder, sphere, cone, torus or wedge",
    ),
    ("part.hole", "Drill holes at a sketch's circles and points"),
    ("part.fillet", "Round edges"),
    ("part.chamfer", "Bevel edges"),
    ("part.draft", "Tilt faces"),
    ("part.thickness", "Hollow the solid"),
    ("part.mirror", "Mirror the last feature"),
    (
        "part.linear_pattern",
        "Repeat the last feature along a line",
    ),
    (
        "part.polar_pattern",
        "Repeat the last feature about an axis",
    ),
    ("part.scaled", "Scale the last feature"),
    ("part.boolean", "Combine with another body"),
];

/// Arguments every feature command reads itself rather than as a field.
const OWN_ARGS: &[&str] = &[
    "sketch",
    "body",
    "name",
    "variant",
    "face_point",
    "face_normal",
];

/// Register every command this module runs.
pub fn register(context: &mut WorkbenchContext) {
    for (id, summary) in FEATURES {
        let mut spec = CommandSpec::new(*id, *summary)
            .optional("sketch", ParamKind::Id, "The sketch it uses")
            .optional(
                "body",
                ParamKind::Id,
                "The body it goes in; the sketch's body when left out",
            )
            .optional("name", ParamKind::String, "Its name in the tree")
            .optional(
                "face_point",
                ParamKind::List,
                "A face it takes as the viewport's picked face (a thickness's \
                 opening, a draft's neutral plane, a mirror's plane): a point of it, {x, y, z}, \
                 in the body's own frame",
            )
            .optional(
                "face_normal",
                ParamKind::List,
                "With face_point: the face's outward normal, {x, y, z}",
            );
        if id.ends_with("primitive") {
            spec = spec.optional(
                "variant",
                ParamKind::String,
                "box (the default), cylinder, sphere, cone, torus or wedge",
            );
        }
        context.register_command(
            spec.extra_args("Any field of the feature, such as length = 20 or reversed = true")
                .returns("the feature's id"),
        );
    }
    context.register_command(
        CommandSpec::new(
            "part.set",
            "Change fields of a Part Design feature or a datum",
        )
        .param("feature", ParamKind::Id, "The feature to change")
        .extra_args("The fields to change, such as length = 25")
        .returns("nothing"),
    );
    context.register_command(
        CommandSpec::new(
            "part.datum",
            "Add a datum plane, line, point or coordinate system",
        )
        .param(
            "kind",
            ParamKind::String,
            "plane, line, point or coordinate_system",
        )
        .param("body", ParamKind::Id, "The body it belongs to")
        .optional(
            "plane",
            ParamKind::String,
            "The base plane it sits on: XY (the default), XZ or YZ",
        )
        .optional(
            "face_point",
            ParamKind::List,
            "Or a flat face it sits on: a point of the face, {x, y, z}",
        )
        .optional(
            "face_normal",
            ParamKind::List,
            "With face_point: the face's outward normal, {x, y, z}",
        )
        .optional(
            "offset",
            ParamKind::List,
            "Moved along its own x, y and normal, {x, y, z} in mm",
        )
        .optional(
            "rotation",
            ParamKind::Number,
            "Turned about its normal, degrees",
        )
        .optional("flip", ParamKind::Bool, "Turned to face the other way")
        .optional("size", ParamKind::Number, "How large it draws, mm")
        .optional("name", ParamKind::String, "Its name in the tree")
        .returns("the datum's id"),
    );
}

/// Run command `id` with `bench` making the features.
pub fn run(
    bench: &PartDesignWorkbench,
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    if id == "part.set" {
        return set(&a, args, ctx);
    }
    if id == "part.datum" {
        return datum(&a, ctx);
    }
    if !FEATURES.iter().any(|(f, _)| *f == id) {
        return Err(CommandError::Unknown(id.to_string()));
    }
    let sketch = a.opt_id("sketch")?.map(FeatureId);
    if let Some(sketch) = sketch {
        let node = ctx
            .document
            .get_feature_meta(sketch)
            .ok_or_else(|| CommandError::bad("sketch", "is not a feature of this document"))?;
        if node.workbench_id.as_str() != "wb.sketch" {
            return Err(CommandError::bad("sketch", "is not a sketch"));
        }
        // The toolbar's path reads the sketch from the selection.
        ctx.active_document_object = Some(sketch);
    }
    let body = match a.opt_id("body")? {
        Some(id) => {
            let body = BodyId(id);
            if !ctx.document.bodies().iter().any(|b| b.id == body) {
                return Err(CommandError::bad("body", "is not a body of this document"));
            }
            body
        }
        None => sketch
            .and_then(|s| ctx.document.get_feature_meta(s).and_then(|n| n.body))
            .or_else(|| ctx.selected_body_id.map(BodyId))
            .ok_or_else(|| CommandError::bad("body", "is required when there is no sketch"))?,
    };
    // A face given here stands in for one picked in the viewport, which is
    // where the toolbar's path reads it, in world space.
    if a.has("face_point") {
        let placement = ctx.document.body_placement(body);
        let point = vector3(a.0.get("face_point"), "face_point")?;
        let normal = vector3(a.0.get("face_normal"), "face_normal")?;
        ctx.selected_face = Some(core_document::FaceRef {
            point: placement.point(point),
            normal: placement.direction(normal),
            surface: None,
        });
    }
    let tool = match a.opt_string("variant")? {
        Some(variant) => format!("{id}:{variant}"),
        None => id.to_string(),
    };
    let fields: Map<String, Value> = args
        .iter()
        .filter(|(k, _)| !OWN_ARGS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let made = bench
        .create_feature(ctx, &tool, body, |feature| apply_fields(feature, &fields))
        .map_err(CommandError::failed)?;
    if let Some(name) = a.opt_string("name")? {
        ctx.document.rename_feature(made.id, name);
    }
    ctx.active_document_object = Some(made.id);
    Ok(json!(made.id.0.to_string()))
}

fn set(a: &Args, args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let id = FeatureId(a.id("feature")?);
    let not_ours = || CommandError::bad("feature", "is not a Part Design feature or a datum");
    let data = ctx.document.get_feature_data(id).ok_or_else(not_ours)?;
    let fields: Map<String, Value> = args
        .iter()
        .filter(|(k, _)| k.as_str() != "feature")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let data = if let Ok(mut feature) = PartFeature::from_json(data) {
        apply_fields(&mut feature, &fields).map_err(CommandError::failed)?;
        feature.to_json()
    } else if let Ok(datum) = DatumFeature::from_json(data) {
        let mut value = datum.to_json();
        merge_fields("Datum", &mut value, &fields).map_err(CommandError::failed)?;
        DatumFeature::from_json(&value)
            .map_err(|e| CommandError::failed(format!("Datum: {e}")))?
            .to_json()
    } else {
        return Err(not_ours());
    };
    ctx.document
        .update_feature_data(id, data)
        .map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document.mark_feature_dirty(id);
    Ok(Value::Null)
}

fn datum(a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let body = BodyId(a.id("body")?);
    if !ctx.document.bodies().iter().any(|b| b.id == body) {
        return Err(CommandError::bad("body", "is not a body of this document"));
    }
    let size = a.opt_number("size")?.map(|s| s as f32);
    let shape = match a.string("kind")? {
        "plane" => DatumShape::Plane {
            size: size.unwrap_or(30.0),
        },
        "line" => DatumShape::Line {
            length: size.unwrap_or(40.0),
        },
        "point" => DatumShape::Point,
        "coordinate_system" => DatumShape::CoordinateSystem {
            size: size.unwrap_or(20.0),
        },
        _ => {
            return Err(CommandError::bad(
                "kind",
                "must be plane, line, point or coordinate_system",
            ));
        }
    };
    let attachment = if a.has("face_point") {
        DatumAttachment::FlatFace {
            point: vector3(a.0.get("face_point"), "face_point")?,
            normal: vector3(a.0.get("face_normal"), "face_normal")?,
        }
    } else {
        DatumAttachment::BasePlane(match a.opt_string("plane")?.unwrap_or("XY") {
            p if p.eq_ignore_ascii_case("XY") => BasePlane::XY,
            p if p.eq_ignore_ascii_case("XZ") => BasePlane::XZ,
            p if p.eq_ignore_ascii_case("YZ") => BasePlane::YZ,
            _ => return Err(CommandError::bad("plane", "must be XY, XZ or YZ")),
        })
    };
    let offset = AttachmentOffset {
        translation: match a.0.get("offset") {
            Some(v) if !v.is_null() => vector3(Some(v), "offset")?,
            _ => [0.0; 3],
        },
        rotation_deg: a.opt_number("rotation")?.unwrap_or(0.0) as f32,
        flip: a.opt_bool("flip")?.unwrap_or(false),
    };
    let name = a
        .opt_string("name")?
        .map(str::to_string)
        .unwrap_or_else(|| PartDesignWorkbench::next_feature_name(ctx, shape.label()));
    let datum = DatumFeature {
        shape,
        attachment,
        offset,
    };
    let id = ctx
        .document
        .add_feature_in_body(datum, name, Some(body))
        .map_err(|e| CommandError::failed(e.to_string()))?;
    Ok(json!(id.0.to_string()))
}

fn vector3(value: Option<&Value>, name: &str) -> Result<[f32; 3], CommandError> {
    let bad = || CommandError::bad(name, "must be {x, y, z}");
    let v = match value {
        Some(Value::Array(v)) if v.len() == 3 => [v[0].as_f64(), v[1].as_f64(), v[2].as_f64()],
        Some(Value::Object(m)) => ["x", "y", "z"].map(|k| m.get(k).and_then(Value::as_f64)),
        _ => return Err(bad()),
    };
    Ok([
        v[0].ok_or_else(bad)? as f32,
        v[1].ok_or_else(bad)? as f32,
        v[2].ok_or_else(bad)? as f32,
    ])
}

/// What a task accepted, as a recording says it: a feature a tool made as
/// the command that makes it, with the fields that differ from what that
/// command would make alone; a datum a tool made as `part.datum`; an edit
/// of an existing one as `part.set` with the fields it changed.
#[cfg(feature = "egui")]
pub(crate) fn record_task(
    bench: &PartDesignWorkbench,
    ctx: &mut WorkbenchRuntimeContext,
    task: &crate::task::TaskState,
) {
    let Some(node) = ctx.document.get_feature_meta(task.feature).cloned() else {
        return;
    };
    let id = json!(task.feature.0.to_string());
    match (&task.made_by, &task.kind) {
        (Some((tool, body)), crate::task::TaskKind::Part) => {
            let command = core_document::base_tool_id(tool);
            if !FEATURES.iter().any(|(f, _)| *f == command) {
                return;
            }
            let fields = inner(&node.data);
            let sketch = fields
                .get("sketch")
                .and_then(Value::as_str)
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .map(FeatureId);
            let default = default_feature(bench, ctx, tool, *body, sketch);
            let mut args = Map::new();
            if let Some(sketch) = sketch {
                args.insert("sketch".into(), json!(sketch.0.to_string()));
            }
            args.insert("body".into(), json!(body.0.to_string()));
            args.insert("name".into(), json!(node.name));
            if let Some(variant) = core_document::tool_variant(tool) {
                args.insert("variant".into(), json!(variant));
            }
            let default_fields = default.as_ref().map(inner).unwrap_or_default();
            for (name, value) in fields {
                if name != "sketch" && default_fields.get(&name) != Some(&value) {
                    args.insert(name, value);
                }
            }
            ctx.record(command, args, id);
        }
        (Some((_, body)), crate::task::TaskKind::Datum) => {
            let Ok(datum) = DatumFeature::from_json(&node.data) else {
                return;
            };
            let (kind, size) = match datum.shape {
                DatumShape::Plane { size } => ("plane", Some(size)),
                DatumShape::Line { length } => ("line", Some(length)),
                DatumShape::Point => ("point", None),
                DatumShape::CoordinateSystem { size } => ("coordinate_system", Some(size)),
            };
            let mut args = json!({
                "kind": kind,
                "body": body.0.to_string(),
                "name": node.name,
                "offset": datum.offset.translation,
                "rotation": datum.offset.rotation_deg,
                "flip": datum.offset.flip,
            });
            if let Some(size) = size {
                args["size"] = json!(size);
            }
            match datum.attachment {
                DatumAttachment::BasePlane(plane) => {
                    args["plane"] = json!(match plane {
                        BasePlane::XY => "XY",
                        BasePlane::XZ => "XZ",
                        BasePlane::YZ => "YZ",
                    });
                }
                DatumAttachment::FlatFace { point, normal } => {
                    args["face_point"] = json!(point);
                    args["face_normal"] = json!(normal);
                }
            }
            ctx.record("part.datum", crate::commands::object(args), id);
        }
        (None, kind) => {
            let (before, after) = match kind {
                crate::task::TaskKind::Part => (inner(&task.snapshot), inner(&node.data)),
                crate::task::TaskKind::Datum => (
                    task.snapshot.as_object().cloned().unwrap_or_default(),
                    node.data.as_object().cloned().unwrap_or_default(),
                ),
            };
            let mut args = Map::new();
            args.insert("feature".into(), id.clone());
            for (name, value) in after {
                if before.get(&name) != Some(&value) {
                    args.insert(name, value);
                }
            }
            if args.len() > 1 {
                ctx.record("part.set", args, Value::Null);
            }
        }
    }
}

/// The fields of a feature's JSON, inside its kind: `{"Pad": {...}}` gives
/// the `{...}`.
fn inner(value: &Value) -> Map<String, Value> {
    value
        .as_object()
        .and_then(|m| m.values().next())
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// A JSON object as named arguments.
pub(crate) fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// The feature `tool` makes for `body` from `sketch` alone, nothing else
/// selected: what the command makes before any field is named.
#[cfg(feature = "egui")]
fn default_feature(
    bench: &PartDesignWorkbench,
    ctx: &mut WorkbenchRuntimeContext,
    tool: &str,
    body: BodyId,
    sketch: Option<FeatureId>,
) -> Option<Value> {
    let saved = (
        ctx.active_document_object,
        ctx.selected_face.take(),
        std::mem::take(&mut ctx.selected_edges),
        ctx.selected_body_id.take(),
    );
    ctx.active_document_object = sketch;
    let made = PartDesignWorkbench::feature_for_tool(
        core_document::base_tool_id(tool),
        core_document::tool_variant(tool),
        ctx,
        body,
    );
    ctx.active_document_object = saved.0;
    ctx.selected_face = saved.1;
    ctx.selected_edges = saved.2;
    ctx.selected_body_id = saved.3;
    made.ok().map(|(mut feature, _)| {
        feature.set_refine(bench.options.refine_result);
        feature.to_json()
    })
}

/// Replace the named fields of the JSON object `value`, refusing a name it
/// does not have.
fn merge_fields(kind: &str, value: &mut Value, fields: &Map<String, Value>) -> Result<(), String> {
    let Value::Object(own) = value else {
        return Err(format!("{kind} has no fields to set"));
    };
    for (name, field) in fields {
        if !own.contains_key(name) {
            let mut known: Vec<&str> = own.keys().map(String::as_str).collect();
            known.sort_unstable();
            return Err(format!(
                "{kind} has no field `{name}`; it has {}",
                known.join(", ")
            ));
        }
        own.insert(name.clone(), field.clone());
    }
    Ok(())
}

/// Replace fields of `feature` with `fields`, refusing a name the feature
/// does not have or a value of the wrong kind.
fn apply_fields(feature: &mut PartFeature, fields: &Map<String, Value>) -> Result<(), String> {
    if fields.is_empty() {
        return Ok(());
    }
    let mut value = serde_json::to_value(&*feature).map_err(|e| e.to_string())?;
    let Some((kind, Value::Object(own))) = value.as_object_mut().and_then(|m| m.iter_mut().next())
    else {
        return Err("this feature has no fields to set".to_string());
    };
    for (name, field) in fields {
        if !own.contains_key(name) {
            let mut known: Vec<&str> = own.keys().map(String::as_str).collect();
            known.sort_unstable();
            return Err(format!(
                "{kind} has no field `{name}`; it has {}",
                known.join(", ")
            ));
        }
        own.insert(name.clone(), field.clone());
    }
    let kind = kind.clone();
    *feature = serde_json::from_value(value).map_err(|e| format!("{kind}: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::{Document, Workbench};

    fn call(
        bench: &mut PartDesignWorkbench,
        doc: &mut Document,
        id: &str,
        args: Value,
    ) -> CommandResult {
        let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        bench.run_command(id, args.as_object().unwrap(), &mut ctx)
    }

    fn sketch_in(doc: &mut Document) -> (BodyId, FeatureId) {
        let body = doc.create_body(None);
        let sketch = doc
            .add_feature_in_body(
                wb_sketch::SketchFeature::new(
                    wb_sketch::sketch::Sketch::new("s"),
                    wb_sketch::sketch::SketchPlane::default(),
                ),
                "sketch".to_string(),
                Some(body),
            )
            .unwrap();
        (body, sketch)
    }

    fn fields(doc: &Document, id: &Value) -> Value {
        let id = FeatureId(uuid::Uuid::parse_str(id.as_str().unwrap()).unwrap());
        doc.get_feature_data(id).unwrap().clone()
    }

    #[test]
    fn a_pad_takes_its_sketch_and_the_fields_named() {
        let mut doc = Document::new("t");
        let (body, sketch) = sketch_in(&mut doc);
        let mut bench = PartDesignWorkbench::default();
        let pad = call(
            &mut bench,
            &mut doc,
            "part.pad",
            json!({"sketch": sketch.0.to_string(), "length": 25.0, "name": "Base"}),
        )
        .unwrap();
        let data = fields(&doc, &pad);
        assert_eq!(data["Pad"]["length"], json!(25.0));
        assert_eq!(data["Pad"]["sketch"], json!(sketch.0.to_string()));
        let id = FeatureId(uuid::Uuid::parse_str(pad.as_str().unwrap()).unwrap());
        let node = doc.get_feature_meta(id).unwrap();
        assert_eq!(node.name, "Base");
        assert_eq!(node.body, Some(body));

        call(
            &mut bench,
            &mut doc,
            "part.set",
            json!({"feature": pad, "length": 40.0, "reversed": true}),
        )
        .unwrap();
        let data = fields(&doc, &pad);
        assert_eq!(data["Pad"]["length"], json!(40.0));
        assert_eq!(data["Pad"]["reversed"], json!(true));
    }

    /// Draft and thickness read their face from the viewport's pick; a
    /// script passes it instead, in the body's own frame, wherever the body
    /// has been moved to.
    #[test]
    fn a_script_gives_a_thickness_or_a_draft_its_face() {
        let mut doc = Document::new("t");
        let (body, sketch) = sketch_in(&mut doc);
        doc.set_body_placement(
            body,
            core_document::BodyPlacement {
                translation: [30.0, -5.0, 2.0],
                rotation: [0.0, 0.0, 0.35f32.sin(), 0.35f32.cos()],
            },
        );
        let mut bench = PartDesignWorkbench::default();
        call(
            &mut bench,
            &mut doc,
            "part.pad",
            json!({"sketch": sketch.0.to_string(), "length": 10.0}),
        )
        .unwrap();
        let face = json!({"face_point": [5.0, 2.5, 10.0], "face_normal": [0.0, 0.0, 1.0]});
        let without = call(
            &mut bench,
            &mut doc,
            "part.thickness",
            json!({"body": body.0.to_string()}),
        );
        assert!(without.is_err(), "no face, no thickness");
        let near = |pick: &Value, want: [f64; 3]| {
            let point: Vec<f64> = pick["point"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();
            point.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-3)
        };
        let mut args = face.clone();
        args["body"] = json!(body.0.to_string());
        let made = call(&mut bench, &mut doc, "part.thickness", args.clone()).unwrap();
        let thickness = fields(&doc, &made);
        let opened = thickness["Thickness"]["faces"].as_array().expect("faces");
        assert_eq!(opened.len(), 1);
        assert!(near(&opened[0], [5.0, 2.5, 10.0]), "{opened:?}");
        // A draft's face is its neutral plane; the faces to tilt are a field.
        args["faces"] = json!([{"point": [10.0, 2.5, 5.0], "normal": [1.0, 0.0, 0.0]}]);
        let made = call(&mut bench, &mut doc, "part.draft", args).unwrap();
        let draft = fields(&doc, &made);
        assert!(
            near(&draft["Draft"]["neutral"], [5.0, 2.5, 10.0]),
            "{draft}"
        );
        assert!(
            near(&draft["Draft"]["faces"][0], [10.0, 2.5, 5.0]),
            "{draft}"
        );
    }

    /// One frame of the task panel, as the host runs it; what it recorded
    /// and the active object it left.
    #[cfg(feature = "egui")]
    fn task_frame(
        bench: &mut PartDesignWorkbench,
        doc: &mut Document,
        active: FeatureId,
        request: core_document::TaskRequest,
    ) -> Vec<core_document::Recorded> {
        let egui_ctx = egui::Context::default();
        ui_kit::apply_theme(&egui_ctx);
        let mut recorded = Vec::new();
        let mut output = egui_ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
            ctx.active_document_object = Some(active);
            bench.ui_task_panel(ui, &mut ctx, request);
            recorded = core_document::HookOutcome::take(&mut ctx).recorded;
        });
        output.textures_delta.clear();
        recorded
    }

    #[cfg(feature = "egui")]
    #[test]
    fn a_pad_made_with_the_tool_records_as_the_command_that_makes_it() {
        let mut doc = Document::new("t");
        let (_, sketch) = sketch_in(&mut doc);
        let before = doc.clone();
        let mut bench = PartDesignWorkbench::default();
        let pad = {
            let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
            ctx.active_document_object = Some(sketch);
            bench.on_input(
                &core_document::WorkbenchInputEvent::KeyPress {
                    key: core_document::KeyCode::A,
                },
                Some("part.pad"),
                &mut ctx,
            );
            ctx.active_document_object.unwrap()
        };
        task_frame(&mut bench, &mut doc, pad, Default::default());
        // The panel's length field, as a person types into it.
        let mut data = doc.get_feature_data(pad).unwrap().clone();
        data["Pad"]["length"] = json!(25.0);
        doc.update_feature_data(pad, data).unwrap();
        let recorded = task_frame(
            &mut bench,
            &mut doc,
            pad,
            core_document::TaskRequest {
                accept: true,
                cancel: false,
            },
        );
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        let call = &recorded[0];
        assert_eq!(call.id, "part.pad");
        assert_eq!(call.args["length"], json!(25.0));
        assert!(
            !call.args.contains_key("reversed"),
            "only what differs from the command's own: {:?}",
            call.args
        );

        // The call makes the same pad on the document as it was.
        let mut replay = before;
        let made = call_on(&mut replay, &call.id, Value::Object(call.args.clone())).unwrap();
        let made = FeatureId(uuid::Uuid::parse_str(made.as_str().unwrap()).unwrap());
        assert_eq!(
            replay.get_feature_data(made),
            doc.get_feature_data(pad),
            "the same fields"
        );
        assert_eq!(
            replay.get_feature_meta(made).unwrap().name,
            doc.get_feature_meta(pad).unwrap().name
        );

        // Editing it again records the change alone.
        task_frame(&mut bench, &mut doc, pad, Default::default());
        let mut data = doc.get_feature_data(pad).unwrap().clone();
        data["Pad"]["reversed"] = json!(true);
        doc.update_feature_data(pad, data).unwrap();
        let edit = task_frame(
            &mut bench,
            &mut doc,
            pad,
            core_document::TaskRequest {
                accept: true,
                cancel: false,
            },
        );
        assert_eq!(edit.len(), 1);
        assert_eq!(edit[0].id, "part.set");
        assert_eq!(
            edit[0].args.len(),
            2,
            "the feature and the one field: {:?}",
            edit[0].args
        );
    }

    fn call_on(doc: &mut Document, id: &str, args: Value) -> CommandResult {
        let mut bench = PartDesignWorkbench::default();
        call(&mut bench, doc, id, args)
    }

    #[test]
    fn an_unknown_field_or_a_wrong_kind_is_refused_and_says_what_there_is() {
        let mut doc = Document::new("t");
        let (_, sketch) = sketch_in(&mut doc);
        let mut bench = PartDesignWorkbench::default();
        let err = call(
            &mut bench,
            &mut doc,
            "part.pad",
            json!({"sketch": sketch.0.to_string(), "colour": "red"}),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("no field `colour`") && err.contains("length"),
            "{err}"
        );
        let err = call(
            &mut bench,
            &mut doc,
            "part.pad",
            json!({"sketch": sketch.0.to_string(), "length": "long"}),
        );
        assert!(err.is_err());
        assert_eq!(
            doc.feature_tree().all_nodes().count(),
            1,
            "a refused command adds nothing"
        );
    }
}
