//! Design's commands: add a feature, or change one, by numbers, for
//! scripts and other callers that are not a click.
//!
//! A feature is made the way its toolbar button makes it, from the sketch
//! and body the call names instead of the selection, with the defaults the
//! button gives; any field of the feature named in the call replaces its
//! default. `pc.doc.feature{id = ...}` shows a feature's fields.

mod bodies;
mod features;
mod history;
mod measure;
mod scan;

#[cfg(feature = "egui")]
use features::FEATURES;
pub(crate) use history::{duplicate, move_feature};
pub(crate) use scan::{SYMMETRY_ACCEPT, SYMMETRY_NAME, scan_symmetry};

use core_document::{
    BodyId, CommandArgs, CommandError, CommandResult, FeatureId, WorkbenchContext,
    WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::{Map, Value, json};

use core_document::{DatumFeature, DatumShape};

use crate::DesignWorkbench;
use crate::feature::DesignFeature;

/// Register every command this module runs.
pub fn register(context: &mut WorkbenchContext) {
    features::configure(context);
    scan::configure(context);
    bodies::configure(context);
    history::configure(context);
    measure::configure(context);
}

/// Run command `id` with `bench` making the features.
pub fn run(
    bench: &DesignWorkbench,
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    if scan::owns(id) {
        return scan::run(bench, id, args, ctx);
    }
    if bodies::owns(id) {
        return bodies::run(id, args, ctx);
    }
    if history::owns(id) {
        return history::run(id, args, ctx);
    }
    if measure::owns(id) {
        return measure::run(id, args, ctx);
    }
    features::run(bench, id, args, ctx)
}

/// A point or vector as Design's features keep it, in singles.
pub(crate) fn vector3(value: Option<&Value>, name: &str) -> Result<[f32; 3], CommandError> {
    Ok(core_document::command::vec3_of(value, name)?.map(|c| c as f32))
}

/// What a task accepted, as a recording says it: a feature a tool made as
/// the command that makes it, with the fields that differ from what that
/// command would make alone; a datum a tool made as `design.datum`; an edit
/// of an existing one as `design.set` with the fields it changed.
#[cfg(feature = "egui")]
pub(crate) fn record_task(
    bench: &DesignWorkbench,
    ctx: &mut WorkbenchRuntimeContext,
    task: &crate::task::TaskState,
) {
    let Some(node) = ctx.document.get_feature_meta(task.feature).cloned() else {
        return;
    };
    let id = json!(task.feature.0.to_string());
    if let Ok(DesignFeature::Borrow {
        source,
        frozen,
        options,
    }) = DesignFeature::from_json(&node.data)
    {
        record_borrow(ctx, task, &node.name, source, frozen.is_some(), options);
        return;
    }
    match (&task.made_by, &task.kind) {
        (Some((tool, body)), crate::task::TaskKind::Part) if tool == "design.split" => {
            let made = DesignFeature::from_json(&node.data)
                .ok()
                .and_then(|f| crate::split::recorded_args(ctx.document, *body, &f));
            if let Some(args) = made {
                let pieces = Value::from(
                    task.pieces
                        .iter()
                        .map(|b| b.0.to_string())
                        .collect::<Vec<_>>(),
                );
                ctx.record("design.split", args, pieces);
            }
        }
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
            // A profile face goes as the face the command takes for one.
            let profile_face = fields
                .get("profile_face")
                .and_then(|v| serde_json::from_value::<crate::FacePick>(v.clone()).ok());
            let placement = ctx.document.body_placement(*body);
            let picked = profile_face.map(|face| core_document::FaceRef {
                name: face.name,
                point: placement.point(face.point),
                normal: placement.direction(face.normal),
                surface: None,
            });
            let default = default_feature(bench, ctx, tool, *body, sketch, picked);
            let mut args = Map::new();
            if let Some(sketch) = sketch {
                args.insert("sketch".into(), json!(sketch.0.to_string()));
            }
            if let Some(face) = profile_face {
                args.insert("face_point".into(), json!(face.point));
                args.insert("face_normal".into(), json!(face.normal));
            }
            args.insert("body".into(), json!(body.0.to_string()));
            args.insert("name".into(), json!(node.name));
            if let Some(variant) = core_document::tool_variant(tool) {
                args.insert("variant".into(), json!(variant));
            }
            let default_fields = default.as_ref().map(inner).unwrap_or_default();
            for (name, value) in fields {
                let own_arg = name == "sketch" || (name == "profile_face" && picked.is_some());
                if !own_arg && default_fields.get(&name) != Some(&value) {
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
            if datum.offset.tilt != [0.0; 2] {
                args["tilt"] = json!(datum.offset.tilt);
            }
            if let Some(size) = size {
                args["size"] = json!(size);
            }
            for (name, value) in crate::datum_refs::attachment_args(&datum.attachment) {
                args[name] = value;
            }
            ctx.record("design.datum", crate::commands::object(args), id);
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
                ctx.record("design.set", args, Value::Null);
            }
        }
    }
}

/// A borrow's task as a recording says it: made, as `design.borrow` with
/// what it borrows; edited, as `design.set` of its source and `design.freeze`
/// when it was frozen, taken again or let go.
#[cfg(feature = "egui")]
fn record_borrow(
    ctx: &mut WorkbenchRuntimeContext,
    task: &crate::task::TaskState,
    name: &str,
    source: crate::feature::BorrowSource,
    frozen: bool,
    options: crate::feature::BorrowOptions,
) {
    use crate::feature::BorrowSource;
    let id = json!(task.feature.0.to_string());
    if let Some((_, body)) = &task.made_by {
        let mut args = Map::new();
        args.insert("body".into(), json!(body.0.to_string()));
        args.insert("name".into(), json!(name));
        match &source {
            BorrowSource::Sketch(sketch) => {
                args.insert("sketch".into(), json!(sketch.0.to_string()));
            }
            BorrowSource::Solid { body, faces, edges } => {
                args.insert("from".into(), json!(body.0.to_string()));
                args.insert("faces".into(), json!(faces));
                args.insert("edges".into(), json!(edges));
            }
        }
        if frozen {
            args.insert("frozen".into(), json!(true));
        }
        if !options.is_plain() {
            args.insert("options".into(), json!(options));
        }
        ctx.record("design.borrow", args, id);
        return;
    }
    let Ok(DesignFeature::Borrow {
        source: source_before,
        frozen: frozen_before,
        options: options_before,
    }) = DesignFeature::from_json(&task.snapshot)
    else {
        return;
    };
    if source_before != source || options_before != options {
        let mut args = Map::new();
        args.insert("feature".into(), id.clone());
        if source_before != source {
            args.insert("source".into(), json!(source));
        }
        if options_before != options {
            args.insert("options".into(), json!(options));
        }
        ctx.record("design.set", args, Value::Null);
    }
    let now = ctx.document.get_feature_data(task.feature).and_then(|d| {
        match DesignFeature::from_json(d).ok()? {
            DesignFeature::Borrow { frozen, .. } => Some(frozen),
            _ => None,
        }
    });
    if now.is_some_and(|now| now != frozen_before) {
        let mut args = Map::new();
        args.insert("feature".into(), id);
        args.insert("frozen".into(), json!(frozen));
        ctx.record("design.freeze", args, Value::Null);
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

/// The feature `tool` makes for `body` from `sketch` (or the world-space
/// `face` picked) alone, nothing else selected: what the command makes
/// before any field is named.
#[cfg(feature = "egui")]
fn default_feature(
    bench: &DesignWorkbench,
    ctx: &mut WorkbenchRuntimeContext,
    tool: &str,
    body: BodyId,
    sketch: Option<FeatureId>,
    face: Option<core_document::FaceRef>,
) -> Option<Value> {
    let saved = (
        ctx.active_document_object,
        std::mem::replace(&mut ctx.selected_face, face),
        std::mem::take(&mut ctx.selected_edges),
        ctx.selected_body_id.take(),
        std::mem::take(&mut ctx.selected_parts),
    );
    ctx.active_document_object = sketch;
    let made = DesignWorkbench::feature_for_tool(
        core_document::base_tool_id(tool),
        core_document::tool_variant(tool),
        ctx,
        body,
        crate::Given::default(),
    );
    ctx.active_document_object = saved.0;
    ctx.selected_face = saved.1;
    ctx.selected_edges = saved.2;
    ctx.selected_body_id = saved.3;
    ctx.selected_parts = saved.4;
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
        own.insert(name.clone(), as_field(&own[name], field));
    }
    Ok(())
}

/// What to write for a field given as a bare name (`cut = "Counterbore"`)
/// whose kind takes values: the form it is written in, its values named
/// as the feature reads them (`cut = {Counterbore = {diameter = …,
/// depth = …}}`). `value` is the feature as `{kind: fields}` with the
/// given fields in.
fn form_hint(value: &Value, kind: &str, fields: &Map<String, Value>) -> Option<String> {
    for (name, given) in fields {
        let Value::String(variant) = given else {
            continue;
        };
        let mut trial = value.clone();
        let mut named: Vec<String> = Vec::new();
        // Each read names the next value the kind is missing.
        for _ in 0..16 {
            let mut inner = Map::new();
            for field in &named {
                inner.insert(field.clone(), json!(0));
            }
            trial[kind][name] = json!({ variant.as_str(): inner });
            let Err(e) = serde_json::from_value::<DesignFeature>(trial.clone()) else {
                break;
            };
            let e = e.to_string();
            match e
                .strip_prefix("missing field `")
                .and_then(|rest| rest.split('`').next())
            {
                Some(field) if !named.iter().any(|n| n == field) => named.push(field.into()),
                _ => break,
            }
        }
        if named.is_empty() {
            continue;
        }
        let values: Vec<String> = named.iter().map(|n| format!("{n} = …")).collect();
        return Some(format!(
            "; `{variant}` takes values, written {name} = {{{variant} = {{{}}}}}",
            values.join(", ")
        ));
    }
    None
}

/// `given` for a field holding `current`: an empty table set on a list is
/// the empty list, since a script's `{}` cannot say which it means.
fn as_field(current: &Value, given: &Value) -> Value {
    match (current, given) {
        (Value::Array(_), Value::Object(map)) if map.is_empty() => Value::Array(Vec::new()),
        _ => given.clone(),
    }
}

/// A feature from its fields as a script gives them, which may say a
/// thing more than one way: `{}` comes as an empty list where a table is
/// wanted, and a datum is `{Datum = id}` or `{Datum = {datum = id}}`
/// wherever a feature takes one. Each way is tried; the first error is the
/// one told.
fn read_fields(value: Value) -> Result<DesignFeature, serde_json::Error> {
    let first = match serde_json::from_value(value.clone()) {
        Ok(read) => return Ok(read),
        Err(first) => first,
    };
    let mut tables = value.clone();
    core_document::command::empty_lists_as_tables(&mut tables);
    if let Ok(read) = serde_json::from_value(tables.clone()) {
        return Ok(read);
    }
    for written in [&value, &tables] {
        for to_table in [true, false] {
            let mut again = written.clone();
            datum_forms(&mut again, to_table);
            if let Ok(read) = serde_json::from_value(again) {
                return Ok(read);
            }
        }
    }
    Err(first)
}

/// Every `{Datum = ...}` in `value` said the other way: an id alone as
/// `{datum = id}` (`to_table`), or `{datum = id}` alone as the id.
fn datum_forms(value: &mut Value, to_table: bool) {
    match value {
        Value::Object(map) => {
            if map.len() == 1
                && let Some(datum) = map.get_mut("Datum")
            {
                match datum {
                    Value::String(id) if to_table => {
                        *datum = json!({"datum": id.clone()});
                        return;
                    }
                    Value::Object(inner) if !to_table && inner.keys().all(|k| k == "datum") => {
                        if let Some(id) = inner.get("datum").cloned() {
                            *datum = id;
                        }
                        return;
                    }
                    _ => {}
                }
            }
            map.values_mut().for_each(|v| datum_forms(v, to_table));
        }
        Value::Array(items) => items.iter_mut().for_each(|v| datum_forms(v, to_table)),
        _ => {}
    }
}

/// Replace fields of `feature` with `fields`, refusing a name the feature
/// does not have or a value of the wrong kind.
fn apply_fields(feature: &mut DesignFeature, fields: &Map<String, Value>) -> Result<(), String> {
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
        let value = as_field(&own[name], field);
        own.insert(name.clone(), value);
    }
    let kind = kind.clone();
    *feature = read_fields(value.clone()).map_err(|first| {
        let hint = form_hint(&value, &kind, fields).unwrap_or_default();
        format!("{kind}: {first}{hint}")
    })?;
    // A Pocket's flag and its ThroughAll mode are one setting: a flag given
    // moves the mode (true to ThroughAll, false back to a plain depth), and
    // the flag then reads what the mode is.
    if let DesignFeature::Pocket {
        through_all, mode, ..
    } = feature
    {
        use crate::feature::ExtrudeMode;
        match fields.get("through_all").and_then(Value::as_bool) {
            Some(true) => *mode = ExtrudeMode::ThroughAll,
            Some(false) if *mode == ExtrudeMode::ThroughAll => *mode = ExtrudeMode::Dimension,
            _ => {}
        }
        *through_all = *mode == ExtrudeMode::ThroughAll;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::made_id;
    use core_document::{Document, Workbench};

    fn call(
        bench: &mut DesignWorkbench,
        doc: &mut Document,
        id: &str,
        args: Value,
    ) -> CommandResult {
        let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        bench.run_command(id, args.as_object().unwrap(), &mut ctx)
    }

    /// A document and the bench its commands run on.
    struct Session {
        doc: Document,
        bench: DesignWorkbench,
    }

    impl Session {
        fn new() -> Self {
            Session {
                doc: Document::new("t"),
                bench: DesignWorkbench::default(),
            }
        }

        fn run(&mut self, id: &str, args: Value) -> CommandResult {
            call(&mut self.bench, &mut self.doc, id, args)
        }

        /// A new body with an empty sketch in it.
        fn sketch(&mut self) -> (BodyId, FeatureId) {
            let body = self.doc.create_body(None);
            (body, self.sketch_in(body, "sketch"))
        }

        fn sketch_in(&mut self, body: BodyId, name: &str) -> FeatureId {
            let sketch = wb_sketch::SketchFeature::new(
                wb_sketch::sketch::Sketch::new(name),
                wb_sketch::sketch::SketchPlane::default(),
            );
            self.doc
                .add_feature_in_body(sketch, name.to_string(), Some(body))
                .unwrap()
        }

        /// The data of the feature a command answered with.
        fn fields(&self, made: &Value) -> Value {
            self.doc.get_feature_data(made_id(made)).unwrap().clone()
        }
    }

    /// A mirror made from the toolbar copies what the tree has picked: one
    /// feature, several, or with the body picked the whole body (no
    /// originals); with nothing picked, the last feature.
    #[test]
    fn a_new_mirror_copies_what_the_tree_picked() {
        let mut s = Session::new();
        let (body, profile) = s.sketch();
        let first = made_id(
            &s.run("design.pad", json!({"sketch": profile, "length": 10.0}))
                .unwrap(),
        );
        let other = s.sketch_in(body, "other");
        let second = made_id(
            &s.run("design.pad", json!({"sketch": other, "length": 5.0}))
                .unwrap(),
        );
        let mut ctx = WorkbenchRuntimeContext::new(&mut s.doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        let mut picked = |active: Option<FeatureId>, several: Vec<FeatureId>, on_body: bool| {
            ctx.active_document_object = active;
            ctx.selected_features = several;
            ctx.selected_body_id = on_body.then_some(body.0);
            DesignWorkbench::mirror_originals(&ctx, body)
        };
        assert_eq!(picked(Some(first), vec![], true), [first]);
        assert_eq!(
            picked(Some(second), vec![second, first], true),
            [first, second]
        );
        assert!(picked(None, vec![], true).is_empty(), "the whole body");
        assert_eq!(picked(None, vec![], false), [second]);
    }

    /// A mirror whose originals are set to none (the whole body) reads no
    /// feature any more, so it moves back past them in the history.
    #[test]
    fn a_mirror_set_to_the_whole_body_moves_earlier() {
        let mut s = Session::new();
        let (body, profile) = s.sketch();
        let first = made_id(
            &s.run("design.pad", json!({"sketch": profile, "length": 10.0}))
                .unwrap(),
        );
        let other = s.sketch_in(body, "other");
        s.run("design.pad", json!({"sketch": other, "length": 5.0}))
            .unwrap();
        let mirror = made_id(
            &s.run("design.mirror", json!({"body": body.0.to_string()}))
                .unwrap(),
        );
        assert!(
            s.doc.move_feature_after(mirror, Some(first)).is_err(),
            "it reads the last pad"
        );
        s.run(
            "design.set",
            json!({"feature": mirror.0.to_string(), "originals": []}),
        )
        .unwrap();
        assert!(s.doc.feature_tree().dependencies(mirror).is_empty());
        s.doc.move_feature_after(mirror, Some(first)).unwrap();
    }

    /// A projection takes its sketch and the faces named, as a point and
    /// normal or as a list, and is refused with neither.
    #[test]
    fn a_projection_takes_its_sketch_and_faces() {
        let mut s = Session::new();
        let (body, profile) = s.sketch();
        s.run("design.pad", json!({"sketch": profile, "length": 10.0}))
            .unwrap();
        let label = s.sketch_in(body, "label");
        let err = s
            .run("design.project", json!({"sketch": label}))
            .unwrap_err();
        assert!(err.to_string().contains("face_point"), "{err}");
        let made = s
            .run(
                "design.project",
                json!({"sketch": label, "face_point": [5.0, 2.5, 10.0],
                       "face_normal": [0.0, 0.0, 1.0]}),
            )
            .unwrap();
        let project = &s.fields(&made)["Project"];
        assert_eq!(project["sketch"], json!(label));
        assert_eq!(project["mode"], "Emboss");
        assert_eq!(project["height"], 1.0);
        assert_eq!(project["faces"].as_array().unwrap().len(), 1);
        let made = s
            .run(
                "design.project",
                json!({"sketch": label, "mode": "Deboss", "height": 0.5,
                       "faces": [{"point": [5.0, 2.5, 10.0], "normal": [0.0, 0.0, 1.0]},
                                 {"point": [10.0, 2.5, 5.0], "normal": [1.0, 0.0, 0.0]}]}),
            )
            .unwrap();
        let project = &s.fields(&made)["Project"];
        assert_eq!(project["mode"], "Deboss");
        assert_eq!(project["faces"].as_array().unwrap().len(), 2);
        assert_eq!(project["wrap"], false);
        let made = s
            .run(
                "design.project",
                json!({"sketch": label, "wrap": true,
                       "face_point": [5.0, 2.5, 10.0], "face_normal": [0.0, 0.0, 1.0]}),
            )
            .unwrap();
        assert_eq!(s.fields(&made)["Project"]["wrap"], true);
    }

    /// A pipe takes the path it is given, from any body; given none it
    /// needs a second sketch in the profile's body, and says so in the
    /// arguments' terms.
    #[test]
    fn a_pipe_takes_a_spine_from_another_body() {
        let mut s = Session::new();
        let (_, profile) = s.sketch();
        let (_, path) = s.sketch();
        let err = s
            .run("design.pipe", json!({"sketch": profile}))
            .unwrap_err();
        assert!(err.to_string().contains("`spine`"), "{err}");
        let made = s
            .run("design.pipe", json!({"sketch": profile, "spine": path}))
            .unwrap();
        assert_eq!(s.fields(&made)["Pipe"]["spine"], json!(path));
    }

    /// A shape the primitive tools do not offer is refused, naming the
    /// ones they do.
    #[test]
    fn an_unknown_primitive_variant_is_refused() {
        let mut s = Session::new();
        let body = s.doc.create_body(None);
        let err = s
            .run("design.primitive", json!({"body": body, "variant": "blob"}))
            .unwrap_err();
        assert!(
            err.to_string().contains("variant") && err.to_string().contains("cylinder"),
            "{err}"
        );
        assert!(s.doc.feature_tree().all_nodes().next().is_none());
    }

    /// A command missing its sketch is told which argument to give, not
    /// what to select in the window.
    #[test]
    fn a_command_refusal_names_the_argument() {
        let mut s = Session::new();
        let body = s.doc.create_body(None);
        let err = s.run("design.revolve", json!({"body": body})).unwrap_err();
        assert!(err.to_string().contains("give `sketch`"), "{err}");
    }

    /// A loft's sketch is its first section: given with more sections it
    /// goes before them, and sections alone take the first as the sketch.
    #[test]
    fn a_loft_s_sketch_leads_its_sections() {
        let mut s = Session::new();
        let (body, a) = s.sketch();
        let b = s.sketch_in(body, "b");
        for args in [
            json!({"sketch": a, "sections": [b]}),
            json!({"sections": [a, b]}),
            json!({"sketch": a, "sections": [a, b]}),
        ] {
            let made = s.run("design.loft", args.clone()).unwrap();
            let sections = s.fields(&made)["Loft"]["sections"].clone();
            assert_eq!(sections, json!([a, b]), "{args}");
        }
    }

    /// A kind that takes values, given by its name alone, is refused with
    /// the form it is written in.
    #[test]
    fn a_bare_name_for_a_kind_with_values_says_how_to_write_it() {
        let mut hole: DesignFeature = serde_json::from_value(json!({"Hole": {
            "sketch": FeatureId::new(), "diameter": 5.0, "depth": 10.0, "through_all": false
        }}))
        .unwrap();
        let error = apply_fields(
            &mut hole,
            json!({"cut": "Counterbore"}).as_object().unwrap(),
        )
        .unwrap_err();
        assert!(
            error.contains("cut = {Counterbore = {diameter = …, depth = …}}"),
            "{error}"
        );
        apply_fields(
            &mut hole,
            json!({"cut": {"Counterbore": {"diameter": 11.0, "depth": 6.0}}})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            hole,
            DesignFeature::Hole {
                cut: crate::feature::HoleCut::Counterbore { .. },
                ..
            }
        ));
    }

    /// A script names a datum either way, and writes `{}` for an empty
    /// table: the fields read all the same.
    #[test]
    fn fields_read_whichever_way_a_script_writes_them() {
        let datum = FeatureId::new();
        let id = datum.0.to_string();
        let mut revolve = DesignFeature::Revolution {
            regions: Vec::new(),
            sketch: FeatureId::new(),
            angle_deg: 360.0,
            axis: crate::feature::RevolveAxis::default(),
            project_axis: false,
            reversed: false,
            midplane: false,
            second_angle_deg: None,
            start_angle_deg: 0.0,
            refine: false,
            boolean_tolerance: None,
            mode: Default::default(),
            up_to_face: None,
        };
        for axis in [json!({"Datum": id}), json!({"Datum": {"datum": id}})] {
            apply_fields(&mut revolve, json!({"axis": axis}).as_object().unwrap()).unwrap();
            assert!(matches!(
                &revolve,
                DesignFeature::Revolution { axis: crate::feature::RevolveAxis::Datum(d), .. } if *d == datum
            ));
        }
        // A mirror's plane is a table: a bare id reads as one too.
        let mut value = json!({"Mirrored": {"originals": [], "plane": {"Datum": id}}});
        datum_forms(&mut value, true);
        assert_eq!(value["Mirrored"]["plane"], json!({"Datum": {"datum": id}}));
        // `{Angled = {}}`: the drill's usual point.
        let point: crate::feature::DrillPoint = {
            let mut v = json!({"Angled": []});
            core_document::command::empty_lists_as_tables(&mut v);
            serde_json::from_value(v).unwrap()
        };
        assert_eq!(
            point,
            crate::feature::DrillPoint::Angled { angle_deg: 118.0 }
        );
    }

    /// A thread is a size alone, or whole; a wrong one says what is taken.
    #[test]
    fn a_thread_reads_as_a_size_or_whole_and_says_what_it_takes() {
        use crate::feature::ThreadSpec;
        use crate::hole_tables::ThreadStandard;
        let m6: ThreadSpec = serde_json::from_value(json!("M6")).unwrap();
        assert_eq!(m6.standard, ThreadStandard::IsoMetricCoarse);
        let unc: ThreadSpec = serde_json::from_value(json!("1/4-20")).unwrap();
        assert_eq!(unc.standard, ThreadStandard::Unc);
        let whole: ThreadSpec =
            serde_json::from_value(json!({"standard": "Unf", "size": "1/4-28"})).unwrap();
        assert_eq!(whole.size, "1/4-28");
        let old: ThreadSpec = serde_json::from_value(json!(5)).unwrap();
        assert_eq!(old.size, "M6");
        let wrong = serde_json::from_value::<ThreadSpec>(json!("M7.3")).unwrap_err();
        assert!(wrong.to_string().contains("IsoMetricCoarse"), "{wrong}");
    }

    #[test]
    fn a_pad_takes_its_sketch_and_the_fields_named() {
        let mut s = Session::new();
        let (body, sketch) = s.sketch();
        let pad = s
            .run(
                "design.pad",
                json!({"sketch": sketch, "length": 25.0, "name": "Base"}),
            )
            .unwrap();
        let data = s.fields(&pad);
        assert_eq!(data["Pad"]["length"], json!(25.0));
        assert_eq!(data["Pad"]["sketch"], json!(sketch));
        let node = s.doc.get_feature_meta(made_id(&pad)).unwrap();
        assert_eq!(node.name, "Base");
        assert_eq!(node.body, Some(body));

        s.run(
            "design.set",
            json!({"feature": pad, "length": 40.0, "reversed": true}),
        )
        .unwrap();
        let data = s.fields(&pad);
        assert_eq!(data["Pad"]["length"], json!(40.0));
        assert_eq!(data["Pad"]["reversed"], json!(true));
    }

    /// A script states a feature's boolean tolerance by its field name,
    /// which then lists as a parameter, and zero puts it back to automatic.
    #[test]
    fn a_script_states_and_clears_a_boolean_tolerance() {
        let mut s = Session::new();
        let (_, sketch) = s.sketch();
        let pad = s
            .run(
                "design.pad",
                json!({"sketch": sketch, "length": 5.0, "boolean_tolerance": 0.05}),
            )
            .unwrap();
        let id = made_id(&pad);
        let read = |doc: &Document| {
            DesignFeature::from_json(doc.get_feature_data(id).unwrap())
                .unwrap()
                .boolean_tolerance()
        };
        assert_eq!(read(&s.doc), Some(0.05));
        let listed = |doc: &Document| {
            crate::params::feature_parameters(doc.get_feature_meta(id).unwrap())
                .iter()
                .any(|p| p.name.as_deref() == Some("boolean_tolerance"))
        };
        assert!(listed(&s.doc), "a stated tolerance is a parameter");

        s.run(
            "design.set",
            json!({"feature": pad, "boolean_tolerance": 0}),
        )
        .unwrap();
        assert_eq!(read(&s.doc), None);
        assert!(!listed(&s.doc), "an automatic tolerance is no parameter");
    }

    /// A pad moved to another body takes its sketch along, and the tree
    /// offers the move for each other body.
    #[test]
    fn a_pad_moves_to_another_body_with_its_sketch() {
        let mut s = Session::new();
        let (from, sketch) = s.sketch();
        let pad = s.run("design.pad", json!({"sketch": sketch})).unwrap();
        let pad_id = made_id(&pad);
        let to = s.doc.create_body(Some("Other".into()));
        let items = s
            .bench
            .menu_items(&core_document::MenuScope::TreeFeature(pad_id), &s.doc);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["Duplicate", "Move to Other"]);

        let moved = s
            .run("design.move_to_body", json!({"feature": pad, "body": to}))
            .unwrap();
        assert_eq!(moved, json!([sketch, pad]));
        assert_eq!(s.doc.get_feature_meta(pad_id).unwrap().body, Some(to));
        assert_eq!(s.doc.get_feature_meta(sketch).unwrap().body, Some(to));
        assert!(crate::design_feature_ids(&s.doc, from).is_empty());
    }

    /// A duplicate reads a copy of its sketch, named as a new feature is,
    /// and the original is untouched.
    #[test]
    fn a_duplicated_pad_reads_its_own_copy_of_the_sketch() {
        let mut s = Session::new();
        let (body, sketch) = s.sketch();
        let pad = s
            .run(
                "design.pad",
                json!({"sketch": sketch, "length": 7.0, "name": "Pad"}),
            )
            .unwrap();
        let made = s.run("design.duplicate", json!({"feature": pad})).unwrap();
        let made: Vec<FeatureId> = made.as_array().unwrap().iter().map(made_id).collect();
        assert_eq!(made.len(), 2, "the sketch and the pad");
        let (new_sketch, new_pad) = (made[0], made[1]);
        let Session { doc, bench } = &mut s;
        assert_ne!(new_sketch, sketch);
        let data = doc.get_feature_data(new_pad).unwrap();
        assert_eq!(data["Pad"]["sketch"], json!(new_sketch));
        assert_eq!(data["Pad"]["length"], json!(7.0));
        assert_eq!(doc.feature_tree().dependencies(new_pad), [new_sketch]);
        assert_eq!(doc.get_feature_meta(new_pad).unwrap().name, "Pad_2");
        assert_eq!(doc.get_feature_meta(new_pad).unwrap().body, Some(body));
        assert_eq!(
            doc.get_feature_meta(new_sketch).unwrap().visible,
            doc.get_feature_meta(sketch).unwrap().visible
        );
        assert_eq!(
            doc.get_feature_data(made_id(&pad)).unwrap()["Pad"]["sketch"],
            json!(sketch)
        );

        // Cut and paste puts it back, in the body selected.
        let other = doc.create_body(None);
        let pad_id = made_id(&pad);
        let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        ctx.active_document_object = Some(pad_id);
        assert!(bench.on_command("edit.cut", &core_document::MenuScope::EditMenu, &mut ctx));
        ctx.active_document_object = None;
        ctx.selected_body_id = Some(other.0);
        assert!(bench.on_command("edit.paste", &core_document::MenuScope::EditMenu, &mut ctx));
        let pasted = ctx.active_document_object.expect("the paste is selected");
        drop(ctx);
        assert!(doc.get_feature_meta(pad_id).is_none(), "cut");
        assert!(doc.get_feature_meta(sketch).is_none(), "with its sketch");
        assert_eq!(doc.get_feature_meta(pasted).unwrap().body, Some(other));
    }

    /// A boolean starts with the body made last, the one just built to
    /// combine, as its tool.
    #[test]
    fn a_boolean_starts_on_the_latest_other_body() {
        let mut s = Session::new();
        let (body, sketch) = s.sketch();
        s.run("design.pad", json!({"sketch": sketch})).unwrap();
        s.doc.create_body(None);
        let latest = s.doc.create_body(None);
        let boolean = s.run("design.boolean", json!({"body": body})).unwrap();
        assert_eq!(
            s.fields(&boolean)["BodyBoolean"]["tool_body"],
            json!(latest)
        );
    }

    /// A datum moves by the same arguments whether it is made or set.
    #[test]
    fn a_datum_is_set_by_the_offset_it_was_made_with() {
        let mut s = Session::new();
        let body = s.doc.create_body(None);
        let datum = s
            .run(
                "design.datum",
                json!({"body": body, "kind": "plane", "offset": [0, 0, 5]}),
            )
            .unwrap();
        s.run(
            "design.set",
            json!({"feature": datum, "offset": {"x": 1, "y": 2, "z": 8}, "rotation": 30}),
        )
        .unwrap();
        let offset = &s.fields(&datum)["offset"];
        assert_eq!(offset["translation"], json!([1.0, 2.0, 8.0]));
        assert_eq!(offset["rotation_deg"], json!(30.0));

        s.run(
            "design.set",
            json!({"feature": datum, "offset":
                {"translation": [0, 0, 3], "rotation_deg": 0, "flip": true}}),
        )
        .unwrap();
        let offset = &s.fields(&datum)["offset"];
        assert_eq!(offset["translation"], json!([0.0, 0.0, 3.0]));
        assert_eq!(offset["flip"], json!(true));
    }

    /// A script's `{}` cannot say it is a list; set on a list field, it is
    /// the empty one.
    #[test]
    fn an_empty_table_empties_a_list_field() {
        let mut s = Session::new();
        let (body, sketch) = s.sketch();
        s.run("design.pad", json!({"sketch": sketch})).unwrap();
        let mirror = s
            .run("design.mirror", json!({"body": body, "originals": {}}))
            .unwrap();
        assert_eq!(s.fields(&mirror)["Mirrored"]["originals"], json!([]));
    }

    /// `through_all` and the ThroughAll mode are one setting: either
    /// spelling sets it, and each reads what the other says.
    #[test]
    fn a_pocket_through_all_reads_back_as_its_mode() {
        let mut s = Session::new();
        let (_, sketch) = s.sketch();
        s.run("design.pad", json!({"sketch": sketch})).unwrap();
        let pocket = s
            .run(
                "design.pocket",
                json!({"sketch": sketch, "through_all": true}),
            )
            .unwrap();
        let data = s.fields(&pocket);
        assert_eq!(data["Pocket"]["mode"], json!("ThroughAll"));
        assert_eq!(data["Pocket"]["through_all"], json!(true));

        s.run(
            "design.set",
            json!({"feature": pocket, "through_all": false}),
        )
        .unwrap();
        let data = s.fields(&pocket);
        assert_eq!(data["Pocket"]["mode"], json!("Dimension"));
        assert_eq!(data["Pocket"]["through_all"], json!(false));
        s.run(
            "design.set",
            json!({"feature": pocket, "mode": "ThroughAll"}),
        )
        .unwrap();
        assert_eq!(s.fields(&pocket)["Pocket"]["through_all"], json!(true));

        s.run(
            "design.set",
            json!({"feature": pocket, "mode": "Dimension", "symmetric": true}),
        )
        .unwrap();
        let data = s.fields(&pocket);
        assert_eq!(data["Pocket"]["mode"], json!("Dimension"));
        assert_eq!(data["Pocket"]["symmetric"], json!(true));
    }

    /// Draft and thickness read their face from the viewport's pick; a
    /// script passes it instead, in the body's own frame, wherever the body
    /// has been moved to.
    #[test]
    fn a_script_gives_a_thickness_or_a_draft_its_face() {
        let mut s = Session::new();
        let (body, sketch) = s.sketch();
        s.doc.set_body_placement(
            body,
            core_document::BodyPlacement {
                translation: [30.0, -5.0, 2.0],
                rotation: [0.0, 0.0, 0.35f32.sin(), 0.35f32.cos()],
            },
        );
        s.run("design.pad", json!({"sketch": sketch, "length": 10.0}))
            .unwrap();
        let without = s.run("design.thickness", json!({"body": body}));
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
        let mut args = json!({"body": body, "face_point": [5.0, 2.5, 10.0],
                              "face_normal": [0.0, 0.0, 1.0]});
        let made = s.run("design.thickness", args.clone()).unwrap();
        let thickness = s.fields(&made);
        let opened = thickness["Thickness"]["faces"].as_array().expect("faces");
        assert_eq!(opened.len(), 1);
        assert!(near(&opened[0], [5.0, 2.5, 10.0]), "{opened:?}");
        // A draft's face is its neutral plane; the faces to tilt are a field.
        args["faces"] = json!([{"point": [10.0, 2.5, 5.0], "normal": [1.0, 0.0, 0.0]}]);
        let made = s.run("design.draft", args).unwrap();
        let draft = s.fields(&made);
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
        bench: &mut DesignWorkbench,
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

    /// A frame that opens the task, then one that accepts it: what the
    /// accepting one recorded.
    #[cfg(feature = "egui")]
    fn open_and_accept(
        bench: &mut DesignWorkbench,
        doc: &mut Document,
        active: FeatureId,
        edit: impl FnOnce(&mut Document),
    ) -> Vec<core_document::Recorded> {
        task_frame(bench, doc, active, Default::default());
        edit(doc);
        let accept = core_document::TaskRequest {
            accept: true,
            cancel: false,
        };
        task_frame(bench, doc, active, accept)
    }

    /// The tool's key pressed with `select` setting what is selected: the
    /// feature it made.
    #[cfg(feature = "egui")]
    fn pad_tool(
        bench: &mut DesignWorkbench,
        doc: &mut Document,
        select: impl FnOnce(&mut WorkbenchRuntimeContext),
    ) -> FeatureId {
        let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        select(&mut ctx);
        bench.on_input(
            &core_document::WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::A,
            },
            Some("design.pad"),
            &mut ctx,
        );
        ctx.active_document_object.unwrap()
    }

    /// `call` run on `replay` makes the feature `made` is in `doc`.
    #[cfg(feature = "egui")]
    fn replays_as(
        replay: &mut Document,
        call: &core_document::Recorded,
        doc: &Document,
        made: FeatureId,
    ) {
        let again = call_on(replay, &call.id, Value::Object(call.args.clone())).unwrap();
        let again = made_id(&again);
        assert_eq!(
            replay.get_feature_data(again),
            doc.get_feature_data(made),
            "the same fields"
        );
        assert_eq!(
            replay.get_feature_meta(again).unwrap().name,
            doc.get_feature_meta(made).unwrap().name
        );
    }

    #[cfg(feature = "egui")]
    #[test]
    fn a_pad_made_with_the_tool_records_as_the_command_that_makes_it() {
        let mut s = Session::new();
        let (_, sketch) = s.sketch();
        let before = s.doc.clone();
        let Session { doc, bench } = &mut s;
        let pad = pad_tool(bench, doc, |ctx| ctx.active_document_object = Some(sketch));
        // The panel's length field, as a person types into it.
        let recorded = open_and_accept(bench, doc, pad, |doc| {
            let mut data = doc.get_feature_data(pad).unwrap().clone();
            data["Pad"]["length"] = json!(25.0);
            doc.update_feature_data(pad, data).unwrap();
        });
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        let call = &recorded[0];
        assert_eq!(call.id, "design.pad");
        assert_eq!(call.args["length"], json!(25.0));
        assert!(
            !call.args.contains_key("reversed"),
            "only what differs from the command's own: {:?}",
            call.args
        );
        // The call makes the same pad on the document as it was.
        replays_as(&mut before.clone(), call, doc, pad);

        // Editing it again records the change alone.
        {
            let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
            ctx.active_document_object = Some(pad);
            core_document::Workbench::edit_feature(bench, &mut ctx, pad);
        }
        let edit = open_and_accept(bench, doc, pad, |doc| {
            let mut data = doc.get_feature_data(pad).unwrap().clone();
            data["Pad"]["reversed"] = json!(true);
            doc.update_feature_data(pad, data).unwrap();
        });
        assert_eq!(edit.len(), 1);
        assert_eq!(edit[0].id, "design.set");
        assert_eq!(
            edit[0].args.len(),
            2,
            "the feature and the one field: {:?}",
            edit[0].args
        );
    }

    fn call_on(doc: &mut Document, id: &str, args: Value) -> CommandResult {
        let mut bench = DesignWorkbench::default();
        call(&mut bench, doc, id, args)
    }

    /// A pad with no sketch takes the face given as its profile, and a pad
    /// the tool made of a picked face records as the call that gives it.
    #[cfg(feature = "egui")]
    #[test]
    fn a_face_profile_pad_is_made_and_recorded_with_its_face() {
        let mut s = Session::new();
        let (body, sketch) = s.sketch();
        s.run("design.pad", json!({"sketch": sketch, "length": 10.0}))
            .unwrap();
        let before = s.doc.clone();
        let made = s
            .run(
                "design.pad",
                json!({"body": body, "face_point": [5.0, 2.5, 10.0],
                    "face_normal": [0.0, 0.0, 1.0], "length": 4.0}),
            )
            .unwrap();
        let data = s.fields(&made);
        assert_eq!(data["Pad"]["sketch"], Value::Null);
        assert_eq!(
            data["Pad"]["profile_face"]["point"],
            json!([5.0, 2.5, 10.0])
        );

        let mut doc = before.clone();
        let pad = pad_tool(&mut s.bench, &mut doc, |ctx| {
            ctx.selected_body_id = Some(body.0);
            ctx.selected_face = Some(core_document::FaceRef {
                name: 0,
                point: [5.0, 2.5, 10.0],
                normal: [0.0, 0.0, 1.0],
                surface: None,
            });
        });
        let recorded = open_and_accept(&mut s.bench, &mut doc, pad, |_| {});
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        let call = &recorded[0];
        assert_eq!(call.args["face_point"], json!([5.0, 2.5, 10.0]));
        assert!(!call.args.contains_key("profile_face"), "{:?}", call.args);
        replays_as(&mut before.clone(), call, &doc, pad);
    }

    #[test]
    fn an_unknown_field_or_a_wrong_kind_is_refused_and_says_what_there_is() {
        let mut s = Session::new();
        let (_, sketch) = s.sketch();
        let err = s
            .run("design.pad", json!({"sketch": sketch, "colour": "red"}))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("no field `colour`") && err.contains("length"),
            "{err}"
        );
        let err = s.run("design.pad", json!({"sketch": sketch, "length": "long"}));
        assert!(err.is_err());
        assert_eq!(
            s.doc.feature_tree().all_nodes().count(),
            1,
            "a refused command adds nothing"
        );
    }
}
