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
const OWN_ARGS: &[&str] = &["sketch", "body", "name", "variant"];

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
            .optional("name", ParamKind::String, "Its name in the tree");
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
