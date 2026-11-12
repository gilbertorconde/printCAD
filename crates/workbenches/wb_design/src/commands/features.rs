//! Making and changing features: one command per feature kind, `design.set`,
//! `design.datum` and `design.borrow`.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandProse, CommandResult, CommandSpec,
    CommandTable, FeatureId, ParamKind, WorkbenchFeature, WorkbenchRuntimeContext,
};
use core_document::{AttachmentOffset, DatumFeature, DatumShape};
use serde_json::{Map, Value, json};
use std::sync::OnceLock;

use super::{apply_fields, merge_fields, vector3};
use crate::DesignWorkbench;
use crate::feature::DesignFeature;

/// The features a command makes, by tool id, and what each is.
pub(super) const FEATURES: &[(&str, &str)] = &[
    ("design.pad", "Pad a sketch"),
    ("design.pocket", "Cut a sketch into the body"),
    ("design.revolve", "Turn a sketch about an axis"),
    ("design.groove", "Cut a sketch turned about an axis"),
    ("design.loft", "Loft through sketches"),
    ("design.subtractive_loft", "Cut a loft through sketches"),
    ("design.pipe", "Sweep a sketch along a path"),
    ("design.subtractive_pipe", "Cut a sketch swept along a path"),
    ("design.helix", "Sweep a sketch along a helix"),
    (
        "design.subtractive_helix",
        "Cut a sketch swept along a helix",
    ),
    (
        "design.primitive",
        "Add a box, cylinder, sphere, cone, torus, ellipsoid, prism, wedge or tube",
    ),
    (
        "design.subtractive_primitive",
        "Cut a box, cylinder, sphere, cone, torus, ellipsoid, prism, wedge or tube",
    ),
    (
        "design.hole",
        "Drill holes at a sketch's circles and points",
    ),
    ("design.fillet", "Round edges"),
    ("design.chamfer", "Bevel edges"),
    ("design.draft", "Tilt faces"),
    ("design.thickness", "Hollow the solid"),
    (
        "design.delete_faces",
        "Delete faces and close the openings from their neighbours",
    ),
    (
        "design.offset_faces",
        "Push or pull faces along their normals, their neighbours following",
    ),
    ("design.scale_body", "Scale the whole solid about a point"),
    (
        "design.offset_solid",
        "Grow or shrink the whole solid by a distance",
    ),
    (
        "design.move_faces",
        "Move or turn faces, their neighbours following",
    ),
    (
        "design.project",
        "Project a sketch onto faces: split lines, emboss or deboss",
    ),
    (
        "design.text",
        "Write text on a face: raised, sunk or imprinted, wrapped onto a curved one",
    ),
    ("design.mirror", "Mirror the last feature"),
    (
        "design.linear_pattern",
        "Repeat the last feature along a line",
    ),
    (
        "design.polar_pattern",
        "Repeat the last feature about an axis",
    ),
    (
        "design.circular_pattern",
        "Repeat the last feature in concentric rings about an axis",
    ),
    (
        "design.point_pattern",
        "Repeat the last feature at a sketch's points",
    ),
    (
        "design.path_pattern",
        "Repeat the last feature along a sketch's curves or an edge",
    ),
    ("design.scaled", "Scale the last feature"),
    ("design.boolean", "Combine with another body"),
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

/// Feature command `id`'s notes and examples, beyond what every feature
/// command says: `features.md`'s, after the one note worked out here.
fn explained(id: &str, spec: CommandSpec) -> CommandSpec {
    let spec = match id {
        "design.fillet" | "design.chamfer" => {
            let (size, round) = if id == "design.fillet" {
                ("`radius`", "rounds")
            } else {
                ("`size`", "bevels")
            };
            spec.note(&format!(
                "Without `edges` it {round} every edge of the solid (`edges = \"All\"`); \
                 {size} is 1 mm when left out. In the app, edges selected in the view \
                 are taken instead."
            ))
        }
        _ => spec,
    };
    prose().explain(spec)
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static CommandProse {
    static PROSE: OnceLock<CommandProse> = OnceLock::new();
    CommandProse::once(&PROSE, "features.md", include_str!("features.md"), &[])
}

fn set(a: &Args, args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let id = FeatureId(a.id("feature")?);
    let not_ours = || CommandError::bad("feature", "is not a Design feature or a datum");
    let data = ctx.document.get_feature_data(id).ok_or_else(not_ours)?;
    let fields: Map<String, Value> = args
        .iter()
        .filter(|(k, _)| k.as_str() != "feature")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    // What a Design feature reads follows its fields: originals or a
    // sketch changed, it reads the new ones, and only them.
    let mut deps = None;
    let mut split = None;
    let data = if let Ok(mut feature) = DesignFeature::from_json(data) {
        let before = feature.clone();
        apply_fields(&mut feature, &fields).map_err(CommandError::failed)?;
        deps = Some(feature.dependencies());
        let data = feature.to_json();
        split = Some((before, feature));
        data
    } else if let Ok(datum) = DatumFeature::from_json(data) {
        let fields = datum_fields(&datum, fields)?;
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
    if let Some(deps) = deps
        && ctx.document.feature_tree().dependencies(id) != deps
    {
        ctx.document.set_feature_dependencies(id, deps);
    }
    if let Some((before, after)) = split {
        crate::split::sync(ctx.document, id, &before, &after);
        if matches!(after, DesignFeature::Text { .. }) {
            crate::text_feature::sync(ctx.document, id).map_err(CommandError::failed)?;
        }
    }
    ctx.document.mark_feature_dirty(id);
    Ok(Value::Null)
}

/// A tilt, {x, y} degrees, where one is given.
fn tilt_of(value: Option<&Value>) -> Result<Option<[f32; 2]>, CommandError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(v) => Ok(Some(
            core_document::command::vec2_of(Some(v), "tilt")?.map(|c| c as f32),
        )),
    }
}

/// A datum's fields as `design.set` takes them: `offset` as `design.datum`
/// gives it, {x, y, z}, with `rotation` and `flip` beside it, or whole as
/// the datum keeps it, {translation, rotation_deg, flip}.
fn datum_fields(
    datum: &DatumFeature,
    mut fields: Map<String, Value>,
) -> Result<Map<String, Value>, CommandError> {
    let mut offset = datum.offset;
    let mut moved = false;
    if let Some(given) = fields.get("offset")
        && given.get("translation").is_none()
    {
        offset.translation = vector3(Some(given), "offset")?;
        moved = true;
    }
    if let Some(rotation) = fields.remove("rotation") {
        offset.rotation_deg = rotation
            .as_f64()
            .ok_or_else(|| CommandError::bad("rotation", "must be a number"))?
            as f32;
        moved = true;
    }
    if let Some(flip) = fields.remove("flip") {
        offset.flip = flip
            .as_bool()
            .ok_or_else(|| CommandError::bad("flip", "must be true or false"))?;
        moved = true;
    }
    if let Some(tilt) = tilt_of(fields.remove("tilt").as_ref())? {
        offset.tilt = tilt;
        moved = true;
    }
    if moved {
        fields.insert("offset".into(), json!(offset));
    }
    Ok(fields)
}

pub(super) fn datum(a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let body = a.body(ctx.document, "body")?;
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
    let attachment = crate::datum_refs::attachment_from_args(a, ctx, body)?;
    let offset = AttachmentOffset {
        tilt: tilt_of(a.0.get("tilt"))?.unwrap_or_default(),
        translation: a
            .opt_vec3("offset")?
            .map_or([0.0; 3], |v| v.map(|c| c as f32)),
        rotation_deg: a.opt_number("rotation")?.unwrap_or(0.0) as f32,
        flip: a.opt_bool("flip")?.unwrap_or(false),
    };
    let name = a
        .opt_string("name")?
        .map(str::to_string)
        .unwrap_or_else(|| DesignWorkbench::next_feature_name(ctx, shape.label()));
    let mut datum = DatumFeature {
        shape,
        attachment,
        offset,
    };
    crate::datum_refs::settle(ctx, body, &mut datum).map_err(CommandError::failed)?;
    let id = ctx
        .document
        .add_feature_in_body(datum, name, Some(body))
        .map_err(|e| CommandError::failed(e.to_string()))?;
    Ok(json!(id.0.to_string()))
}

/// `design.borrow`: a borrow of a sketch, or of faces and edges, added to a
/// body as a feature of its history.
fn borrow(a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    use crate::feature::{BorrowSource, EdgePick, FacePick};
    let body = a.body(ctx.document, "body")?;
    // An imported solid takes a base shape first, and builds on it.
    if ctx.document.body_solid_is_imported(body) && crate::take_base(ctx, body).is_none() {
        return Err(CommandError::bad(
            "body",
            "takes its shape from elsewhere (a mesh, a copy or a linked part) and takes no \
             features; borrow into a body of its own",
        ));
    }
    let source = match (a.opt_id("sketch")?, a.opt_id("from")?) {
        (Some(sketch), None) => BorrowSource::Sketch(FeatureId(sketch)),
        (None, Some(from)) => {
            let list = |name: &str| match a.0.get(name) {
                None | Some(Value::Null) => Ok(Vec::new()),
                Some(Value::Array(items)) => Ok(items.clone()),
                // A script's empty table.
                Some(Value::Object(map)) if map.is_empty() => Ok(Vec::new()),
                Some(_) => Err(CommandError::bad(name, "must be a list")),
            };
            let faces = list("faces")?
                .iter()
                .map(|f| {
                    let point = vector3(f.get("point"), "faces")?;
                    Ok(FacePick {
                        // Named as a pick names it, so the borrow keeps the
                        // face as the lender changes.
                        name: crate::borrow::face_name_at(ctx.document, BodyId(from), point),
                        point,
                        normal: vector3(f.get("normal"), "faces")?,
                    })
                })
                .collect::<Result<Vec<_>, CommandError>>()?;
            let edges = list("edges")?
                .iter()
                .map(|e| {
                    Ok(EdgePick {
                        faces: [0, 0],
                        point: vector3(e.get("point"), "edges")?,
                        direction: vector3(e.get("direction"), "edges")?,
                    })
                })
                .collect::<Result<Vec<_>, CommandError>>()?;
            BorrowSource::Solid {
                body: BodyId(from),
                faces,
                edges,
            }
        }
        _ => {
            return Err(CommandError::bad(
                "sketch",
                "give a sketch, or from with the faces and edges it lends, not both",
            ));
        }
    };
    if let Some(why) = crate::borrow::refusal(ctx.document, body, &source) {
        let arg = if matches!(source, BorrowSource::Sketch(_)) {
            "sketch"
        } else {
            "from"
        };
        return Err(CommandError::bad(arg, why));
    }
    let options: crate::feature::BorrowOptions = match a.0.get("options") {
        None | Some(Value::Null) => Default::default(),
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|e| CommandError::bad("options", e.to_string()))?,
    };
    if (options.fill || options.whole) && matches!(source, BorrowSource::Sketch(_)) {
        return Err(CommandError::bad(
            "options",
            "fill and whole take from a solid, not a sketch",
        ));
    }
    let frozen = if a.opt_bool("frozen")?.unwrap_or(false) {
        Some(
            crate::borrow::freeze(ctx.document, ctx.kernel, body, &source, &options)
                .map_err(CommandError::failed)?,
        )
    } else {
        None
    };
    let name = a
        .opt_string("name")?
        .map(str::to_string)
        .unwrap_or_else(|| DesignWorkbench::next_feature_name(ctx, "Borrowed"));
    let feature = DesignFeature::Borrow {
        source,
        frozen,
        options,
    };
    let id = ctx
        .document
        .add_feature_in_body(feature, name, Some(body))
        .map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document.mark_feature_dirty(id);
    ctx.active_document_object = Some(id);
    Ok(json!(id.0.to_string()))
}

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<DesignWorkbench>) {
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
                 opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a \
                 pocket given no sketch): a point of it, {x, y, z}, in the body's own frame",
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
                "box (the default), cylinder, sphere, cone, torus, ellipsoid, prism, wedge or tube",
            );
        }
        t.add_on_by_id(
            explained(
                id,
                spec.extra_args("Any field of the feature, such as length = 20 or reversed = true")
                    .returns("the feature's id")
                    .note(
                        "It makes the feature and builds nothing: a feature that cannot build \
                         is told by `pc.doc.rebuild()`, in the list it returns, and its body \
                         stays the solid before it. A misspelt field is refused here, naming the \
                         fields the feature has.",
                    )
                    .see_also("doc.rebuild")
                    .see_also("design.set"),
            ),
            make,
        );
    }
    t.add(
        prose().explain(
            CommandSpec::new("design.set", "Change fields of a Design feature or a datum")
                .param("feature", ParamKind::Id, "The feature to change")
                .extra_args(
                    "The fields to change, such as length = 25; a datum takes offset {x, y, z}, \
             rotation and flip as design.datum does",
                )
                .returns("nothing"),
        ),
        |args, ctx| set(&Args(args), args, ctx),
    );
    t.add(
        prose().explain(
            CommandSpec::new(
                "design.datum",
                "Add a datum plane, line, point or coordinate system",
            )
            .param(
                "kind",
                ParamKind::String,
                "plane, line, point or coordinate_system",
            )
            .param("body", ParamKind::Id, "The body it belongs to")
            .optional(
                "mode",
                ParamKind::String,
                "What it attaches to: base_plane (the default), face, three_points, \
             normal_to_edge, along_edge, two_points, plane_intersection, midplane, \
             curve_centre or inertia; references are in the body's own frame and follow its solid",
            )
            .optional(
                "plane",
                ParamKind::String,
                "base_plane: the base plane it sits on, XY (the default), XZ or YZ",
            )
            .optional(
                "face_point",
                ParamKind::List,
                "face: a point of the face, {x, y, z}; without a mode, a flat face kept as given",
            )
            .optional(
                "face_normal",
                ParamKind::List,
                "With face_point: the face's outward normal, {x, y, z}",
            )
            .optional(
                "edge_point",
                ParamKind::List,
                "normal_to_edge, along_edge, curve_centre: a point of the edge, {x, y, z}",
            )
            .optional(
                "edge_direction",
                ParamKind::List,
                "With edge_point: the way the edge runs there, {x, y, z}",
            )
            .optional(
                "spot",
                ParamKind::String,
                "normal_to_edge: where on the edge, picked (the default), start, end, middle \
             or centre",
            )
            .optional(
                "points",
                ParamKind::List,
                "three_points, two_points: each {x, y, z}, or {face_point, face_normal}, or \
             {edge_point, edge_direction, spot}",
            )
            .optional(
                "planes",
                ParamKind::List,
                "plane_intersection, midplane: two of XY, XZ, YZ, a datum's id, or \
             {face_point, face_normal}",
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
            .optional(
                "tilt",
                ParamKind::List,
                "Tilted about its own x-axis, then its y-axis, {x, y} in degrees",
            )
            .optional("size", ParamKind::Number, "How large it draws, mm")
            .optional("name", ParamKind::String, "Its name in the tree")
            .returns("the datum's id"),
        ),
        |args, ctx| datum(&Args(args), ctx),
    );
    t.add(
        prose().explain(
            CommandSpec::new(
                "design.borrow",
                "Borrow another body's sketch, or faces and edges of its solid",
            )
            .param("body", ParamKind::Id, "The body that borrows")
            .optional(
                "sketch",
                ParamKind::Id,
                "A sketch of another body: its profile, for this body's features",
            )
            .optional(
                "from",
                ParamKind::Id,
                "Or the body whose solid lends faces and edges",
            )
            .optional(
                "faces",
                ParamKind::List,
                "With from: faces it lends, each {point, normal} in that body's own frame",
            )
            .optional(
                "edges",
                ParamKind::List,
                "With from: edges it lends, each {point, direction} in that body's own frame",
            )
            .optional(
                "frozen",
                ParamKind::Bool,
                "Keep the geometry as it is now rather than follow the source",
            )
            .optional(
                "options",
                ParamKind::Any,
                "How it lends: {offset = {translation = {x, y, z}, rotation_deg, tilt = {x, y}, \
             flip}} moves and turns it along and about this body's axes; fill = true makes \
             closed borrowed edges a face features take as a profile; whole = true lends the \
             whole solid's edges as reference",
            )
            .optional("name", ParamKind::String, "Its name in the tree")
            .returns("the borrow's id"),
        ),
        |args, ctx| borrow(&Args(args), ctx),
    );
}

/// Make the feature command `id` makes, as its toolbar button does.
fn make(
    bench: &mut DesignWorkbench,
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let mut sketch = a.opt_id("sketch")?.map(FeatureId);
    // A loft's sketch is its first section: given only sections, the first
    // is that sketch.
    let lofting = matches!(id, "design.loft" | "design.subtractive_loft");
    let listed: Vec<FeatureId> = match args.get("sections") {
        Some(Value::Array(list)) if lofting => list
            .iter()
            .filter_map(|v| v.as_str().and_then(|s| uuid::Uuid::parse_str(s).ok()))
            .map(FeatureId)
            .collect(),
        _ => Vec::new(),
    };
    if sketch.is_none() {
        sketch = listed.first().copied();
    }
    // A borrowed face as the profile: its borrow stands selected, as the
    // toolbar reads it.
    if let Some(borrow) = args
        .get("profile_borrowed")
        .and_then(|r| r.get("borrow"))
        .and_then(Value::as_str)
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
    {
        ctx.active_document_object = Some(FeatureId(borrow));
    }
    if let Some(sketch) = sketch {
        let node = ctx
            .document
            .get_feature_meta(sketch)
            .ok_or_else(|| CommandError::bad("sketch", "is not a feature of this document"))?;
        if node.workbench_id.as_str() != wb_sketch::KIND
            && !crate::borrow::lends_sketch(ctx.document, sketch)
        {
            return Err(CommandError::bad("sketch", "is not a sketch"));
        }
        // The toolbar's path reads the sketch from the selection.
        ctx.active_document_object = Some(sketch);
    }
    let body = match a.opt_body(ctx.document, "body")? {
        Some(body) => body,
        None => sketch
            .and_then(|s| ctx.document.get_feature_meta(s).and_then(|n| n.body))
            .or_else(|| ctx.selected_body_id.map(BodyId))
            .ok_or_else(|| CommandError::bad("body", "is required when there is no sketch"))?,
    };
    // A face given here, in the body's own frame, stands in for one picked
    // in the viewport: placed in world space, where the toolbar's path
    // reads it.
    if a.has("face_point") {
        let placement = ctx.document.body_placement(body);
        let point = vector3(a.0.get("face_point"), "face_point")?;
        let normal = vector3(a.0.get("face_normal"), "face_normal")?;
        ctx.selected_face = Some(core_document::FaceRef {
            name: 0,
            point: placement.point(point),
            normal: placement.direction(normal),
            surface: None,
        });
    }
    // A thickness opens a face, a draft turns about one, and a deletion,
    // offset or move works on one: in a script that face is an argument.
    if matches!(
        id,
        "design.thickness"
            | "design.draft"
            | "design.delete_faces"
            | "design.offset_faces"
            | "design.move_faces"
            | "design.text"
    ) && ctx.selected_face.is_none()
    {
        return Err(CommandError::bad(
            "face_point",
            "is required with face_normal: a point on the face (the one to open or to \
             delete, or the neutral plane) and its outward normal, in the body's own frame",
        ));
    }
    if id == "design.project" && ctx.selected_face.is_none() && !a.has("faces") {
        return Err(CommandError::bad(
            "face_point",
            "is required with face_normal, or `faces` in its place: the faces the sketch \
             is projected onto, each a point on it and its outward normal, in the body's \
             own frame",
        ));
    }
    let tool = match a.opt_string("variant")? {
        Some(variant) if !crate::PRIMITIVE_SHAPES.iter().any(|(v, _)| *v == variant) => {
            let names: Vec<&str> = crate::PRIMITIVE_SHAPES.iter().map(|(v, _)| *v).collect();
            return Err(CommandError::bad(
                "variant",
                format!("is not a shape: give one of {}", names.join(", ")),
            ));
        }
        Some(variant) => format!("{id}:{variant}"),
        None => id.to_string(),
    };
    let mut fields: Map<String, Value> = args
        .iter()
        .filter(|(k, _)| !OWN_ARGS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if lofting
        && let (Some(sketch), Some(Value::Array(list))) = (sketch, fields.get_mut("sections"))
        && !listed.contains(&sketch)
    {
        list.insert(0, json!(sketch.0.to_string()));
    }
    let made = bench
        .create_feature(
            ctx,
            &tool,
            body,
            crate::Given {
                scripted: true,
                spine: a.opt_id("spine").ok().flatten().map(FeatureId),
            },
            |feature| apply_fields(feature, &fields),
        )
        .map_err(CommandError::failed)?;
    if let Some(name) = a.opt_string("name")? {
        ctx.document.rename_feature(made.id, name);
    }
    ctx.active_document_object = Some(made.id);
    Ok(json!(made.id.0.to_string()))
}
