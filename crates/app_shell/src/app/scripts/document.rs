//! Document command handlers shared by desktop and headless execution.

use crate::ui::TreeItemId;
use core_document::{Args, BodyId, CommandArgs, CommandError, CommandResult, FeatureId};
use serde_json::{Value, json};
use uuid::Uuid;

/// What a quantity is called in a command's answer.
fn kind_name(dim: core_document::expr::Dim) -> String {
    use core_document::expr::Dim;
    match dim {
        Dim::LENGTH => "length".into(),
        Dim::ANGLE => "angle".into(),
        Dim::NUMBER => "number".into(),
        Dim::AREA => "area".into(),
        Dim::VOLUME => "volume".into(),
        other => other.unit_text("mm"),
    }
}

/// A slot's value as a command answers it.
fn slot_json(
    result: &Result<core_document::expr::Quantity, String>,
    unit: core_document::Unit,
) -> Value {
    match result {
        Ok(q) => json!({
            "value": q.value,
            "kind": kind_name(q.dim),
            "text": q.display(unit, 4),
            "error": Value::Null,
        }),
        Err(why) => json!({"value": Value::Null, "text": Value::Null, "error": why}),
    }
}

/// A variable set named or given by id.
fn set_arg(document: &core_document::Document, a: &Args) -> Result<FeatureId, CommandError> {
    let given = a.string("set")?;
    let by_id = Uuid::parse_str(given).ok().map(FeatureId).filter(|id| {
        document
            .get_feature_meta(*id)
            .is_some_and(|n| n.workbench_id.as_str() == core_document::VARIABLES_KIND)
    });
    by_id
        .or_else(|| {
            document.object_named(given).filter(|id| {
                document
                    .get_feature_meta(*id)
                    .is_some_and(|n| n.workbench_id.as_str() == core_document::VARIABLES_KIND)
            })
        })
        .ok_or_else(|| CommandError::failed(format!("no variable set is called {given}")))
}

/// What slot `key` of feature `id` comes to now.
fn slot_answer(
    document: &mut core_document::Document,
    registry: &core_document::DocumentService,
    id: FeatureId,
    key: &str,
) -> Value {
    registry.evaluate(document);
    let unit = document.display_unit();
    document
        .evaluated_slots(id)
        .iter()
        .find(|s| s.key == key)
        .map(|s| slot_json(&s.result, unit))
        .unwrap_or(Value::Null)
}

/// What `doc.measure`'s `about` names: "origin", a point {x, y, z}, a line
/// {point, direction}, or a datum point, coordinate system or line by id,
/// placed in the world.
fn inertia_reference(
    document: &core_document::Document,
    value: &Value,
) -> Result<crate::ui::InertiaReference, CommandError> {
    use crate::ui::InertiaReference as R;
    use core_document::WorkbenchFeature;
    let bad = || {
        CommandError::bad(
            "about",
            "is \"origin\", {x, y, z}, {point, direction} or a datum's id",
        )
    };
    let xyz = |v: &Value| -> Option<[f64; 3]> {
        let at = |key: &str, i: usize| v.get(key).or_else(|| v.get(i)).and_then(Value::as_f64);
        Some([at("x", 0)?, at("y", 1)?, at("z", 2)?])
    };
    if value.as_str() == Some("origin") {
        return Ok(R::Point {
            name: "origin".into(),
            at: [0.0; 3],
        });
    }
    if let Some(id) = value.as_str().and_then(|s| uuid::Uuid::parse_str(s).ok()) {
        let id = FeatureId(id);
        let node = document
            .get_feature_meta(id)
            .filter(|n| n.workbench_id.as_str() == core_document::DATUM_KIND)
            .ok_or_else(|| CommandError::bad("about", "is no datum"))?;
        let datum = document
            .feature_values(id)
            .and_then(|v| core_document::DatumFeature::from_json(v).ok())
            .ok_or_else(|| CommandError::failed("the datum could not be read"))?;
        let placement = node
            .body
            .map(|body| document.body_placement(body))
            .unwrap_or_default();
        let frame = datum.frame();
        let origin = placement.point(frame.origin).map(f64::from);
        return match datum.shape {
            core_document::DatumShape::Line { .. } => Ok(R::Axis {
                name: node.name.clone(),
                point: origin,
                direction: placement.direction(frame.x_axis).map(f64::from),
            }),
            core_document::DatumShape::Plane { .. } => Err(CommandError::bad(
                "about",
                "is a datum plane; give a point or a line",
            )),
            _ => Ok(R::Point {
                name: node.name.clone(),
                at: origin,
            }),
        };
    }
    if let (Some(point), Some(direction)) = (value.get("point"), value.get("direction")) {
        let (point, direction) = (xyz(point).ok_or_else(bad)?, xyz(direction).ok_or_else(bad)?);
        if direction == [0.0; 3] {
            return Err(CommandError::bad(
                "about",
                "a line's direction has no length",
            ));
        }
        return Ok(R::Axis {
            name: "line".into(),
            point,
            direction,
        });
    }
    xyz(value)
        .map(|at| R::Point {
            name: "point".into(),
            at,
        })
        .ok_or_else(bad)
}

/// The commands every host of a document answers the same way, with or
/// without a window: reading it, naming and adding bodies, and measuring.
/// `None` for any other command.
/// `doc.print_layout`: the layout of `document`'s parts on the bed the
/// arguments give, else on `printing`'s.
pub(crate) fn print_layout_command(
    args: &CommandArgs,
    document: &core_document::Document,
    registry: &core_document::DocumentService,
    printing: &settings::PrintingSettings,
) -> CommandResult {
    use crate::app::print_layout::{LayoutSettings, lay_out, parts_to_print, piece_mesh};
    let a = Args(args);
    let mut settings = LayoutSettings::of(printing);
    if let Some(gap) = a.opt_number("gap")? {
        if gap < 0.0 {
            return Err(CommandError::bad("gap", "must not be negative"));
        }
        settings.gap_mm = gap as f32;
    }
    if let Some(bed) = args.get("bed") {
        let size = |key: &str, at: usize| {
            bed.get(key)
                .or_else(|| bed.get(at))
                .and_then(Value::as_f64)
                .filter(|v| *v > 0.0)
                .ok_or_else(|| CommandError::bad("bed", "is {x, y, z}, each above 0"))
        };
        settings.bed_mm = [
            size("x", 0)? as f32,
            size("y", 1)? as f32,
            size("z", 2)? as f32,
        ];
    }
    let layout = lay_out(document, &parts_to_print(document, registry), &settings);
    let pieces: Vec<Value> = layout
        .pieces
        .iter()
        .map(|piece| {
            let bounds = piece_mesh(document, piece).and_then(|m| m.bounds());
            json!({
                "body": piece.body.0.to_string(),
                "name": piece.name,
                "plate": piece.plate + 1,
                "min": bounds.map(|b| b.0),
                "max": bounds.map(|b| b.1),
                "transform": piece.placement.rows(),
            })
        })
        .collect();
    Ok(json!({
        "plates": layout.plates,
        "pieces": pieces,
        "too_big": layout.too_big,
        "too_tall": layout.too_tall,
        "unbuilt": layout.unbuilt,
    }))
}

pub(crate) fn document_command(
    id: &str,
    args: &CommandArgs,
    document: &mut core_document::Document,
    registry: &core_document::DocumentService,
    file: Option<&std::path::Path>,
) -> Option<CommandResult> {
    if let Some(answer) = crate::app::section_commands::document_section_command(id, args, document)
    {
        return Some(answer);
    }
    let a = Args(args);
    let answer = (|| match id {
        "doc.agent_rules" => Ok(json!(document.agent_rules())),
        "doc.set_agent_rules" => {
            document.set_agent_rules(a.string("rules")?);
            Ok(Value::Null)
        }
        "doc.info" => Ok(json!({
            "name": document.name(),
            "file": file.map(|p| p.display().to_string()),
            "unit": document.display_unit().short_label(),
            "modified": document.metadata().dirty(),
        })),
        "doc.bodies" => Ok(Value::Array(
            document
                .bodies()
                .iter()
                .map(|b| {
                    json!({
                        "id": b.id.0.to_string(),
                        "name": b.name,
                        "visible": !b.hidden,
                        "frozen": b.frozen,
                        "scan": b.scan.map(|s| json!({
                            "units": crate::app::scans::unit_key(
                                s.read_in.unwrap_or(kernel_api::LengthUnit::Millimetre),
                            ),
                        })),
                        "selectable": !b.unselectable,
                        "material": b.material.as_ref().map(|m| json!({
                            "name": m.name,
                            "density": m.density,
                        })),
                        "array": document
                            .array_instance(b.id)
                            .map(|(array, _)| array.id.0.to_string()),
                        "features": features_in_order(document, Some(b.id))
                            .iter()
                            .map(|n| n.id.0.to_string())
                            .collect::<Vec<_>>(),
                    })
                })
                .collect(),
        )),
        "doc.features" => {
            let body = a.opt_id("body")?.map(BodyId);
            Ok(Value::Array(
                features_in_order(document, None)
                    .into_iter()
                    .filter(|n| body.is_none() || n.body == body)
                    .map(|n| {
                        json!({
                            "id": n.id.0.to_string(),
                            "name": n.name,
                            "kind": kind_of(registry, n),
                            "body": n.body.map(|b| b.0.to_string()),
                            "visible": n.visible,
                            "suppressed": n.suppressed,
                            "error": n.error,
                        })
                    })
                    .collect(),
            ))
        }
        "doc.feature" => {
            let node = document
                .get_feature_meta(FeatureId(a.id("id")?))
                .ok_or_else(|| CommandError::bad("id", "is not a feature of this document"))?;
            Ok(json!({
                "id": node.id.0.to_string(),
                "name": node.name,
                "kind": kind_of(registry, node),
                "body": node.body.map(|b| b.0.to_string()),
                "visible": node.visible,
                "suppressed": node.suppressed,
                "error": node.error,
                "fields": node.data,
                "unset": unset_fields(&node.data),
                // What it is built from now, where that is not what is kept:
                // formulas worked out, a plane where what it stands on is.
                "values": document
                    .feature_values(node.id)
                    .filter(|v| **v != node.data)
                    .cloned(),
            }))
        }
        "doc.suppress" => {
            let feature = feature_arg(document, &a)?;
            suppress(document, feature, a.opt_bool("suppressed")?.unwrap_or(true));
            Ok(Value::Null)
        }
        "doc.move" => {
            let feature = feature_arg(document, &a)?;
            let up = a.opt_bool("up")?.unwrap_or(true);
            move_in_history(document, feature, up).map_err(CommandError::failed)?;
            Ok(json!(true))
        }
        "doc.set_tip" => {
            let feature = feature_arg(document, &a)?;
            let tip = (!a.opt_bool("clear")?.unwrap_or(false)).then_some(feature);
            set_tip(document, registry, feature, tip).map_err(CommandError::failed)?;
            Ok(Value::Null)
        }
        "doc.coupon" => {
            let numbers = |key: &str, default: Vec<f64>| -> Result<Vec<f64>, CommandError> {
                match a.0.get(key) {
                    None | Some(Value::Null) => Ok(default),
                    Some(Value::Array(list)) => list
                        .iter()
                        .map(|v| {
                            v.as_f64()
                                .ok_or_else(|| CommandError::bad(key, "is a list of numbers"))
                        })
                        .collect(),
                    Some(Value::Object(map)) if map.is_empty() => Ok(Vec::new()),
                    Some(_) => Err(CommandError::bad(key, "is a list of numbers")),
                }
            };
            let defaults = bench_fixtures::CouponSizes::default();
            let sizes = bench_fixtures::CouponSizes {
                diameters: numbers("diameters", defaults.diameters)?,
                gaps: numbers("gaps", defaults.gaps)?,
                overhangs: numbers("overhangs", defaults.overhangs)?,
            };
            let c = bench_fixtures::coupon(document, &sizes).map_err(CommandError::failed)?;
            let at = |list: &[([f64; 3], f64)], what: &str| -> Value {
                list.iter()
                    .map(|(p, v)| json!({"at": {"x": p[0], "y": p[1], "z": p[2]}, what: v}))
                    .collect()
            };
            Ok(json!({
                "body": c.body.0.to_string(),
                "width": c.width,
                "depth": c.depth,
                "holes": at(&c.holes, "diameter"),
                "pins": at(&c.pins, "diameter"),
                "gaps": at(&c.gaps, "width"),
                "overhangs": at(&c.overhangs, "angle"),
            }))
        }
        "doc.new_body" => {
            let body = document.create_body(None);
            if let Some(name) = a.opt_string("name")? {
                document.rename_body(body, name);
            }
            Ok(json!(body.0.to_string()))
        }
        "doc.set_body" => {
            let body = body_arg(document, &a)?;
            set_body(document, registry, body, &a)?;
            Ok(Value::Null)
        }
        "doc.set_textures" => {
            let body = body_arg(document, &a)?;
            let textures = textures_arg(document, body, a.0.get("textures"))?;
            document.set_body_textures(body, textures);
            Ok(Value::Null)
        }
        "doc.set_face_color" => {
            let body = body_arg(document, &a)?;
            let index = a.number("face")? as i64;
            let mesh = document
                .imported_geometry(body)
                .map(|g| std::sync::Arc::clone(&g.mesh))
                .ok_or_else(|| CommandError::failed("the body has no solid yet"))?;
            let faces = mesh.faces.iter().map(|f| *f as i64 + 1).max().unwrap_or(0);
            if index < 0 || index >= faces {
                return Err(CommandError::bad(
                    "face",
                    format!("is not one of its {faces} faces"),
                ));
            }
            let color = match a.0.get("color") {
                None | Some(Value::Null) => None,
                Some(v) => Some(color_arg(v, "color")?),
            };
            document.color_face(body, index as u32, color);
            Ok(Value::Null)
        }
        "doc.linked_copy" => {
            let body = body_arg(document, &a)?;
            let copy = document
                .create_linked_copy(body, None)
                .ok_or_else(|| CommandError::failed("that body cannot be copied"))?;
            // Beside the original, clear of it along X.
            let width = document
                .imported_geometry(body)
                .and_then(|g| g.bounds_mm.or_else(|| g.mesh.bounds()))
                .map_or(10.0, |(lo, hi)| hi[0] - lo[0]);
            let at = document.body_placement(body);
            document.set_body_placement(
                copy,
                core_document::BodyPlacement::new(
                    at.quat(),
                    at.offset() + glam::Vec3::X * (width + 10.0),
                ),
            );
            Ok(json!(copy.0.to_string()))
        }
        "doc.array" => {
            let component = a.opt_id("component")?.map(core_document::ComponentId);
            let body = match component {
                Some(c) => *document
                    .component_bodies(c)
                    .first()
                    .ok_or_else(|| array_refusal(&core_document::ArrayError::EmptyComponent))?,
                None => body_arg(document, &a)?,
            };
            let given = count_arg(&a)?;
            let pattern = array_pattern(document, registry, body, given.unwrap_or(1), None, &a)?;
            let count = array_count(given, &pattern)?;
            let id = match component {
                Some(c) => document.create_component_array(c, pattern.clone(), count),
                None => document.create_copy_array(body, pattern.clone(), count),
            }
            .map_err(|e| array_refusal(&e))?;
            if let Some(skip) = skip_arg(&a)? {
                document
                    .set_copy_array(id, pattern, count, skip)
                    .map_err(|e| array_refusal(&e))?;
            }
            Ok(array_answer(document, id))
        }
        "doc.set_array" => {
            let id = core_document::ArrayId(a.id("array")?);
            let array = document
                .copy_array(id)
                .cloned()
                .ok_or_else(|| CommandError::bad("array", "is not an array of this document"))?;
            if a.opt_bool("remove")?.unwrap_or(false) {
                document.remove_copy_array(id);
                return Ok(json!({"id": id.0.to_string(), "instances": []}));
            }
            let given = count_arg(&a)?;
            let count = given.unwrap_or(array.count);
            let pattern = array_pattern(
                document,
                registry,
                array.source,
                count,
                Some(&array.pattern),
                &a,
            )?;
            let count = match pattern {
                core_document::ArrayPattern::Points { .. } => array_count(given, &pattern)?,
                _ => count,
            };
            let skip = skip_arg(&a)?.unwrap_or(array.skipped);
            document
                .set_copy_array(id, pattern, count, skip)
                .map_err(|e| array_refusal(&e))?;
            Ok(array_answer(document, id))
        }
        "doc.move_after" => {
            let feature = FeatureId(a.id("id")?);
            let after = a.opt_id("after")?.map(FeatureId);
            move_after(document, feature, after).map_err(CommandError::failed)?;
            Ok(Value::Null)
        }
        "doc.recompute" => {
            let body = body_arg(document, &a)?;
            if document.body_frozen(body) {
                return Err(CommandError::bad(
                    "body",
                    "is frozen: it keeps the solid it has until it is thawed \
                     (doc.set_body{frozen = false})",
                ));
            }
            registry.invalidate_body(document, body);
            Ok(Value::Null)
        }
        "doc.replace_shape" => {
            let body = body_arg(document, &a)?;
            let path = a.string("path")?;
            let bytes = crate::platform::read(std::path::Path::new(path))
                .map_err(|e| CommandError::bad("path", format!("could not be read: {e}")))?;
            if !document.replace_body_shape(body, path, bytes) {
                return Err(CommandError::bad(
                    "body",
                    "builds its shape from its history, or takes it from another body or \
                     file; only an imported or converted shape, or a base shape, is replaced",
                ));
            }
            Ok(Value::Null)
        }
        "doc.rename" => {
            let name = a.string("name")?.to_string();
            let item = tree_item(document, a.id("id")?)?;
            user_note_guard(document, item, &a, "reword")?;
            match item {
                TreeItemId::Body(body) => document.rename_body(body, name),
                TreeItemId::Feature(feature) => document.rename_feature(feature, name),
                TreeItemId::Component(id) => {
                    if let Some(mut component) = document.component(id).cloned() {
                        component.name = name;
                        document
                            .update_component(component)
                            .map_err(|e| CommandError::failed(e.to_string()))?;
                    }
                }
                TreeItemId::Note(id) => {
                    if let Some((_, note)) = document.note(id) {
                        let mut note = note.clone();
                        note.text = name;
                        document.set_note(note);
                    }
                }
                _ => return Err(CommandError::bad("id", "cannot be renamed")),
            }
            Ok(Value::Null)
        }
        "doc.note_add" => {
            let body = body_arg(document, &a)?;
            let text = a.string("text")?.to_string();
            let point = match a.0.get("point") {
                None | Some(Value::Null) => None,
                Some(v) => Some(numbers_arg(v, "point", ["x", "y", "z"])?),
            };
            let want = match a.opt_string("on")? {
                None | Some("face") => core_document::NoteWant::Face,
                Some("edge") => core_document::NoteWant::Edge(None),
                Some("point") => core_document::NoteWant::Point,
                Some(_) => return Err(CommandError::bad("on", "must be face, edge or point")),
            };
            let mut note = document.new_note(body, text, note_author(&a)?, point, want);
            note.asks = a.opt_bool("asks")? == Some(true);
            if let Some(label) = a.0.get("label").filter(|v| !v.is_null()) {
                let label = numbers_arg(label, "label", ["x", "y", "z"])?;
                note.label = document.body_placement(body).inverse().point(label);
            }
            let id = document
                .add_note(body, note)
                .ok_or_else(|| CommandError::bad("body", "is not a body of this document"))?;
            Ok(json!(id.0.to_string()))
        }
        "doc.note_set" => {
            let id = note_arg(document, &a)?;
            let (body, note) = document
                .note(id)
                .ok_or_else(|| CommandError::bad("id", "is not a note of this document"))?;
            let mut note = note.clone();
            if let Some(text) = a.opt_string("text")? {
                if note_author(&a)? == core_document::NoteAuthor::Agent
                    && note.author == core_document::NoteAuthor::User
                    && text != note.text
                {
                    return Err(CommandError::failed(
                        "the text of a user's note is the user's: an agent may resolve or \
                         answer it, not reword it",
                    ));
                }
                note.text = text.to_string();
            }
            if let Some(label) = a.0.get("label").filter(|v| !v.is_null()) {
                let label = numbers_arg(label, "label", ["x", "y", "z"])?;
                note.label = document.body_placement(body).inverse().point(label);
            }
            if let Some(visible) = a.opt_bool("visible")? {
                note.hidden = !visible;
            }
            if let Some(done) = a.opt_string("done")? {
                note.resolved = Some(done.to_string());
            }
            if a.opt_bool("reopen")? == Some(true) {
                note.resolved = None;
            }
            document.set_note(note);
            Ok(Value::Null)
        }
        "doc.note_remove" => {
            if !document.remove_note(note_arg(document, &a)?) {
                return Err(CommandError::bad("id", "is not a note of this document"));
            }
            Ok(Value::Null)
        }
        "doc.notes" => {
            let only = a.opt_id("body")?.map(BodyId);
            Ok(Value::Array(
                document
                    .notes()
                    .filter(|(body, _)| only.is_none_or(|o| o == *body))
                    .map(|(body, note)| crate::app::note_tool::note_json(document, body, note))
                    .collect(),
            ))
        }
        "doc.notes_open" => Ok(Value::Array(
            document
                .notes()
                .filter(|(_, note)| note.resolved.is_none())
                .map(|(body, note)| crate::app::note_tool::note_json(document, body, note))
                .collect(),
        )),
        "doc.faces" | "doc.edges" => {
            let body = body_arg(document, &a)?;
            let in_body = match a.opt_string("frame")? {
                None | Some("world") => false,
                Some("body") => true,
                Some(_) => return Err(CommandError::bad("frame", "must be world or body")),
            };
            let mesh = if in_body {
                document.local_geometry(body).map(|(mesh, _)| mesh)
            } else {
                document
                    .imported_geometry(body)
                    .map(|g| std::sync::Arc::clone(&g.mesh))
            }
            .ok_or_else(|| CommandError::failed("the body has no solid yet"))?;
            Ok(if id == "doc.faces" {
                faces_of(&mesh)
            } else {
                crate::app::edges::edges_of(&mesh)
            })
        }
        "doc.measure" => {
            let body = body_arg(document, &a)?;
            let (min, max) = document
                .imported_geometry(body)
                .and_then(|g| g.bounds_mm.or_else(|| g.mesh.bounds()))
                .ok_or_else(|| CommandError::failed("the body has no solid yet"))?;
            let blob = document.imported_brep_blob_arc(body).ok_or_else(|| {
                CommandError::failed("the body is a mesh; it has no solid to measure")
            })?;
            let props = kernel_ogeom::OgeomKernel::new()
                .physical_properties(&blob)
                .map_err(|e| CommandError::failed(e.to_string()))?;
            let c = props.centre_mm.map(|v| v as f32);
            let centre = document.body_placement(body).point(c);
            let about = match args.get("about").filter(|v| !v.is_null()) {
                Some(value) => Some(inertia_reference(document, value)?),
                None => None,
            };
            let principal = |found: [(f64, [f64; 3]); 3]| {
                found.map(|(moment, axis)| json!({"moment": moment, "axis": axis}))
            };
            let centre_wide = centre.map(f64::from);
            let volume = props.volume_mm3.unwrap_or(0.0);
            let inertia = props.inertia.map(|inertia| {
                let turned = core_document::inertia::turned(
                    &inertia,
                    document.body_placement(body).quat().as_dquat(),
                );
                let about = about.as_ref().map(|about| match about {
                    crate::ui::InertiaReference::Axis {
                        point, direction, ..
                    } => json!({
                        "point": point,
                        "direction": direction,
                        "moment": core_document::inertia::about_axis(
                            &turned.about_centre,
                            volume,
                            centre_wide,
                            *point,
                            *direction,
                        ),
                    }),
                    crate::ui::InertiaReference::Point { at, .. } => {
                        let tensor = core_document::inertia::about_point(
                            &turned.about_centre,
                            volume,
                            centre_wide,
                            *at,
                        );
                        json!({
                            "point": at,
                            "tensor": tensor,
                            "principal": principal(core_document::inertia::principal(&tensor)),
                        })
                    }
                    crate::ui::InertiaReference::CentreOfMass => json!({
                        "point": centre_wide,
                        "tensor": turned.about_centre,
                        "principal": principal(turned.principal),
                    }),
                });
                json!({
                    "tensor": turned.about_centre,
                    "principal": principal(turned.principal),
                    "about": about,
                })
            });
            Ok(json!({
                "volume": props.volume_mm3,
                "area": props.area_mm2,
                "centre": centre,
                "min": min,
                "max": max,
                "approximate": props.approximate,
                "inertia": inertia,
            }))
        }
        "doc.parameters" => {
            let feature = FeatureId(a.id("id")?);
            let node = document
                .get_feature_meta(feature)
                .ok_or_else(|| CommandError::failed("no such feature"))?
                .clone();
            registry.evaluate(document);
            let unit = document.display_unit();
            let slots = document.evaluated_slots(feature);
            Ok(Value::Array(
                registry
                    .parameters(&node)
                    .into_iter()
                    .map(|p| {
                        let mut row = slots
                            .iter()
                            .find(|s| s.key == p.key)
                            .map(|s| slot_json(&s.result, unit))
                            .unwrap_or_else(|| json!({}));
                        row["name"] = json!(p.name);
                        row["key"] = json!(p.key);
                        row["label"] = json!(p.label);
                        row["kind"] = json!(kind_name(p.dim));
                        row["formula"] = json!(node.formulas.get(&p.key));
                        row
                    })
                    .collect(),
            ))
        }
        "doc.set_formula" => {
            let feature = FeatureId(a.id("id")?);
            let parameter = registry
                .parameter_named(document, feature, a.string("parameter")?)
                .map_err(CommandError::failed)?;
            let formula = a.opt_string("formula")?.map(str::to_string);
            match &formula {
                Some(text) => core_document::expr::check_syntax(text)
                    .map_err(|e| CommandError::failed(e.message))?,
                // What the formula comes to now is the number kept.
                None => registry.evaluate(document),
            }
            document
                .set_feature_formula(feature, parameter.key.clone(), formula)
                .map_err(|e| CommandError::failed(e.to_string()))?;
            Ok(slot_answer(document, registry, feature, &parameter.key))
        }
        "doc.set_value" => {
            let feature = FeatureId(a.id("id")?);
            let parameter = registry
                .parameter_named(document, feature, a.string("parameter")?)
                .map_err(CommandError::failed)?;
            registry
                .set_parameter_value(document, feature, &parameter, a.number("value")?)
                .map(|()| Value::Null)
                .map_err(CommandError::failed)
        }
        "var.new" => document
            .add_variable_set(a.string("name")?)
            .map(|id| json!(id.0.to_string()))
            .map_err(CommandError::failed),
        "var.set" => {
            let set = set_arg(document, &a)?;
            let name = a.string("name")?.to_string();
            document
                .set_variable(set, &name, a.string("formula")?, a.opt_string("comment")?)
                .map_err(CommandError::failed)?;
            Ok(slot_answer(document, registry, set, &name))
        }
        "var.remove" => {
            let set = set_arg(document, &a)?;
            document
                .remove_variable(set, a.string("name")?)
                .map(|()| Value::Null)
                .map_err(CommandError::failed)
        }
        "var.rename" => {
            let set = set_arg(document, &a)?;
            document
                .rename_variable(set, a.string("name")?, a.string("to")?)
                .map(|()| Value::Null)
                .map_err(CommandError::failed)
        }
        "var.list" => {
            let only = match a.opt_string("set")? {
                Some(_) => Some(set_arg(document, &a)?),
                None => None,
            };
            registry.evaluate(document);
            let unit = document.display_unit();
            Ok(Value::Array(
                document
                    .variable_sets()
                    .into_iter()
                    .filter(|(id, ..)| only.is_none_or(|o| o == *id))
                    .map(|(id, name, set)| {
                        let slots = document.evaluated_slots(id);
                        let variables: Vec<Value> = set
                            .variables
                            .iter()
                            .map(|v| {
                                let mut row = slots
                                    .iter()
                                    .find(|s| s.key == v.name)
                                    .map(|s| slot_json(&s.result, unit))
                                    .unwrap_or_else(|| json!({}));
                                row["name"] = json!(v.name);
                                row["formula"] = json!(v.formula);
                                row["comment"] = json!(v.comment);
                                row
                            })
                            .collect();
                        json!({"id": id.0.to_string(), "name": name, "variables": variables})
                    })
                    .collect(),
            ))
        }
        "config.list" => {
            let table = document
                .configurations()
                .map(|(_, t)| t)
                .unwrap_or_default();
            Ok(json!({
                "columns": table.columns,
                "rows": table.rows.iter().map(|r| json!({
                    "name": r.name,
                    "values": r.values,
                    "left_out": r.left_out.iter().map(|b| b.0.to_string()).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "active": table.active,
            }))
        }
        "config.new" => document
            .add_configuration(a.string("name")?, a.opt_string("like")?)
            .map(|()| Value::Null)
            .map_err(CommandError::failed),
        "config.remove" => document
            .remove_configuration(a.string("name")?)
            .map(|()| Value::Null)
            .map_err(CommandError::failed),
        "config.rename" => document
            .rename_configuration(a.string("name")?, a.string("to")?)
            .map(|()| Value::Null)
            .map_err(CommandError::failed),
        "config.add_variable" => document
            .add_configuration_column(a.string("variable")?)
            .map(|()| Value::Null)
            .map_err(CommandError::failed),
        "config.remove_variable" => document
            .remove_configuration_column(a.string("variable")?)
            .map(|()| Value::Null)
            .map_err(CommandError::failed),
        "config.set" => document
            .set_configuration_value(a.string("name")?, a.string("variable")?, a.string("value")?)
            .map(|()| Value::Null)
            .map_err(CommandError::failed),
        "config.leave_out" => {
            let bodies =
                a.0.get("bodies")
                    .and_then(Value::as_array)
                    .ok_or_else(|| CommandError::bad("bodies", "must be a list of ids"))?
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .and_then(|t| Uuid::parse_str(t).ok())
                            .map(core_document::BodyId)
                            .ok_or_else(|| CommandError::bad("bodies", "must be a list of ids"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
            document
                .set_configuration_left_out(a.string("name")?, bodies)
                .map(|()| Value::Null)
                .map_err(CommandError::failed)
        }
        "config.activate" => document
            .activate_configuration(a.opt_string("name")?)
            .map(|()| Value::Null)
            .map_err(CommandError::failed),
        "var.eval" => {
            let q = registry
                .evaluate_formula(document, a.string("formula")?, None)
                .map_err(CommandError::failed)?;
            Ok(json!({
                "value": q.value,
                "kind": kind_name(q.dim),
                "text": q.display(document.display_unit(), 4),
            }))
        }
        _ => Err(CommandError::Unknown(id.to_string())),
    })();
    match answer {
        Err(CommandError::Unknown(_)) => None,
        answer => Some(answer),
    }
}

fn feature_arg(document: &core_document::Document, a: &Args) -> Result<FeatureId, CommandError> {
    let feature = FeatureId(a.id("id")?);
    if document.get_feature_meta(feature).is_some() {
        Ok(feature)
    } else {
        Err(CommandError::bad("id", "is not a feature of this document"))
    }
}

/// Leave `feature` out of its body's solid, or put it back.
pub(crate) fn suppress(document: &mut core_document::Document, feature: FeatureId, on: bool) {
    document.set_feature_suppressed(feature, on);
    document.mark_feature_dirty(feature);
}

/// Suppress or unsuppress each of `features`; the caller closes the
/// gesture once, so they are one undo step.
pub(crate) fn suppress_all(
    document: &mut core_document::Document,
    features: &[FeatureId],
    on: bool,
) {
    for feature in features {
        suppress(document, *feature, on);
    }
}

/// The `doc.suppress` call that suppresses `feature`, or puts it back.
pub(crate) fn suppress_call(feature: FeatureId, on: bool) -> core_document::Recorded {
    let mut args = CommandArgs::new();
    args.insert("id".into(), json!(feature.0.to_string()));
    args.insert("suppressed".into(), json!(on));
    core_document::Recorded {
        id: "doc.suppress".to_string(),
        args,
        result: Value::Null,
    }
}

/// Move `feature` a step earlier (`up`) or later, or say why it cannot go.
pub(crate) fn move_in_history(
    document: &mut core_document::Document,
    feature: FeatureId,
    up: bool,
) -> Result<(), String> {
    use core_document::MoveRefused;
    let name = |document: &core_document::Document, id: FeatureId| {
        document
            .get_feature_meta(id)
            .map(|n| n.name.clone())
            .unwrap_or_default()
    };
    document
        .try_move_feature_in_history(feature, up)
        .map_err(|refused| match refused {
            MoveRefused::NotFound => "no such feature".to_string(),
            MoveRefused::AtEnd if up => {
                format!("{} is already first in its body", name(document, feature))
            }
            MoveRefused::AtEnd => {
                format!("{} is already last in its body", name(document, feature))
            }
            MoveRefused::Dependency { neighbour } => {
                let (this, other) = (name(document, feature), name(document, neighbour));
                if up {
                    format!("{this} cannot go before {other}, which it is built from")
                } else {
                    format!("{this} cannot go after {other}, which is built from it")
                }
            }
        })
}

/// Build the body of `of` only up to `tip`, or all of it when `None`.
pub(crate) fn set_tip(
    document: &mut core_document::Document,
    registry: &core_document::DocumentService,
    of: FeatureId,
    tip: Option<FeatureId>,
) -> Result<(), &'static str> {
    let body = document
        .get_feature_meta(of)
        .and_then(|n| n.body)
        .ok_or("the feature belongs to no body")?;
    set_body_tip(document, registry, body, tip);
    Ok(())
}

/// Build `body` only up to `tip`, or all of it when `None`.
pub(crate) fn set_body_tip(
    document: &mut core_document::Document,
    registry: &core_document::DocumentService,
    body: core_document::BodyId,
    tip: Option<FeatureId>,
) {
    document.set_body_tip(body, tip);
    // The chain changes shape: rebuild from the first feature.
    registry.invalidate_body(document, body);
}

/// A colour given as {r, g, b} (or a list of three), each 0 to 1.
fn color_arg(value: &Value, name: &str) -> Result<[f32; 3], CommandError> {
    let part = |key: &str, i: usize| {
        value
            .get(key)
            .or_else(|| value.get(i))
            .and_then(Value::as_f64)
            .map(|v| v.clamp(0.0, 1.0) as f32)
    };
    match (part("r", 0), part("g", 1), part("b", 2)) {
        (Some(r), Some(g), Some(b)) => Ok([r, g, b]),
        _ => Err(CommandError::bad(name, "is {r, g, b}, each from 0 to 1")),
    }
}

/// Numbers given as a table of named parts or a list, in `keys` order.
fn numbers_arg<const N: usize>(
    value: &Value,
    name: &str,
    keys: [&str; N],
) -> Result<[f32; N], CommandError> {
    let mut out = [0.0f32; N];
    for (i, key) in keys.iter().enumerate() {
        out[i] = value
            .get(*key)
            .or_else(|| value.get(i))
            .and_then(Value::as_f64)
            .ok_or_else(|| CommandError::bad(name, format!("is {{{}}}", keys.join(", "))))?
            as f32;
    }
    Ok(out)
}

/// What `doc.set_body` changes, each named field in turn.
pub(super) fn set_body(
    document: &mut core_document::Document,
    registry: &core_document::DocumentService,
    body: BodyId,
    a: &Args,
) -> Result<(), CommandError> {
    let entry = document
        .bodies()
        .iter()
        .find(|b| b.id == body)
        .cloned()
        .ok_or_else(|| CommandError::bad("body", "is not a body of this document"))?;
    let color = a.0.get("color");
    let opacity = a.opt_number("opacity")?;
    if color.is_some() || opacity.is_some() {
        let display = match color {
            Some(Value::Null) if opacity.is_none() => None,
            _ => {
                let mut display = entry.display.unwrap_or_default();
                if let Some(v) = color.filter(|v| !v.is_null()) {
                    display.color = color_arg(v, "color")?;
                }
                if let Some(o) = opacity {
                    display.opacity = o.clamp(0.05, 1.0) as f32;
                }
                Some(display)
            }
        };
        document.set_body_display(body, display);
    }
    match a.0.get("material") {
        None => {}
        Some(Value::Null) => document.set_body_material(body, None),
        Some(v) => {
            let name = v
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Material")
                .to_string();
            let density = v
                .get("density")
                .and_then(Value::as_f64)
                .filter(|d| *d > 0.0)
                .ok_or_else(|| CommandError::bad("material", "needs a density above 0, g/cm³"))?;
            document.set_body_material(
                body,
                Some(core_document::Material {
                    name,
                    density: density as f32,
                }),
            );
        }
    }
    if let Some(shading) = a.opt_string("shading")? {
        let crease = a.opt_number("crease")?.unwrap_or(30.0);
        if !(0.0..=180.0).contains(&crease) {
            return Err(CommandError::bad(
                "crease",
                "is an angle from 0 to 180 degrees",
            ));
        }
        document.set_body_shading(
            body,
            match shading {
                "as_meshed" => core_document::Shading::AsMeshed,
                "sharp" => core_document::Shading::Sharp,
                "smooth" => core_document::Shading::Smooth {
                    crease_deg: crease as f32,
                },
                _ => {
                    return Err(CommandError::bad(
                        "shading",
                        "is as_meshed, sharp or smooth",
                    ));
                }
            },
        );
    }
    if let Some(frozen) = a.opt_bool("frozen")? {
        document.set_body_frozen(body, frozen);
        if !frozen && entry.frozen {
            // What changed while it was frozen builds now.
            registry.invalidate_body(document, body);
        }
    }
    if let Some(selectable) = a.opt_bool("selectable")? {
        document.set_body_selectable(body, selectable);
    }
    if let Some(scan) = a.opt_bool("scan")? {
        let info = scan.then(|| entry.scan.unwrap_or_default());
        if scan && !document.is_mesh_body(body) {
            return Err(CommandError::bad(
                "scan",
                "only a mesh body (an STL, OBJ, 3MF, PLY … not yet converted) can be a scan",
            ));
        }
        document.set_body_scan(body, info);
    }
    match a.0.get("face_colors") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) if list.is_empty() => {
            for c in &entry.face_colors {
                document.set_face_color(body, c.index, c.name, None);
            }
        }
        Some(_) => {
            return Err(CommandError::bad(
                "face_colors",
                "takes only an empty list; doc.set_face_color colours a face",
            ));
        }
    }
    let translation = a.0.get("translation").filter(|v| !v.is_null());
    let rotation = a.0.get("rotation").filter(|v| !v.is_null());
    if translation.is_some() || rotation.is_some() {
        let now = document.body_placement(body);
        let offset = match translation {
            Some(v) => glam::Vec3::from_array(numbers_arg(v, "translation", ["x", "y", "z"])?),
            None => now.offset(),
        };
        let turn = match rotation {
            Some(v) => glam::Quat::from_array(numbers_arg(v, "rotation", ["x", "y", "z", "w"])?)
                .normalize(),
            None => now.quat(),
        };
        document.place_with_unit(body, core_document::BodyPlacement::new(turn, offset));
    }
    Ok(())
}

/// The feature a drop next to `target` lands after: `target` itself, or
/// for a drop before it, the one before it in its body's history
/// (`Some(None)`: the start). `None` when `target` is in no history.
pub(crate) fn drop_place(
    document: &core_document::Document,
    target: FeatureId,
    before: bool,
) -> Option<Option<FeatureId>> {
    let order = document.body_history_of(target);
    let at = order.iter().position(|f| *f == target)?;
    Some(if before {
        at.checked_sub(1).map(|i| order[i])
    } else {
        Some(target)
    })
}

/// Move `feature` in its body's history to just after `after`, or first
/// for `None`, saying why a step is refused.
pub(crate) fn move_after(
    document: &mut core_document::Document,
    feature: FeatureId,
    after: Option<FeatureId>,
) -> Result<(), String> {
    use core_document::MoveRefused;
    let name = |document: &core_document::Document, id: FeatureId| {
        document
            .get_feature_meta(id)
            .map(|n| n.name.clone())
            .unwrap_or_default()
    };
    document
        .move_feature_after(feature, after)
        .map_err(|refused| match refused {
            MoveRefused::NotFound => "the two features are not in one body".to_string(),
            MoveRefused::AtEnd => "it can go no further".to_string(),
            MoveRefused::Dependency { neighbour } => format!(
                "{} and {} depend on each other and keep their order",
                name(document, feature),
                name(document, neighbour)
            ),
        })
}

/// A body's textures as a script gives them: each face a number (as
/// `doc.faces` has it) or `{name, index}` as a recording writes it.
fn textures_arg(
    document: &core_document::Document,
    body: BodyId,
    value: Option<&Value>,
) -> Result<Vec<core_document::FaceTexture>, CommandError> {
    let list = value
        .and_then(Value::as_array)
        .ok_or_else(|| CommandError::bad("textures", "must be a list"))?;
    let mesh = document
        .imported_geometry(body)
        .map(|g| std::sync::Arc::clone(&g.mesh));
    list.iter()
        .map(|item| {
            let texture: surface_texture::Texture =
                serde_json::from_value(item.get("texture").cloned().unwrap_or(Value::Null))
                    .map_err(|e| {
                        CommandError::bad("textures", format!("has a texture it cannot read: {e}"))
                    })?;
            let faces = item
                .get("faces")
                .and_then(Value::as_array)
                .map(|faces| {
                    faces
                        .iter()
                        .map(|face| match face.as_u64() {
                            Some(index) => mesh
                                .as_deref()
                                .map(|m| core_document::FaceKey::of(m, index as u32))
                                .ok_or_else(|| CommandError::failed("the body has no solid yet")),
                            None => serde_json::from_value(face.clone()).map_err(|e| {
                                CommandError::bad(
                                    "textures",
                                    format!("has a face it cannot read: {e}"),
                                )
                            }),
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?
                .unwrap_or_default();
            Ok(core_document::FaceTexture { texture, faces })
        })
        .collect()
}

/// An array command's `count`, when given.
fn count_arg(a: &Args) -> Result<Option<u32>, CommandError> {
    match a.opt_number("count")? {
        None => Ok(None),
        Some(n) if n >= 1.0 && n <= f64::from(core_document::arrays::MAX_ARRAY_COUNT) => {
            Ok(Some(n as u32))
        }
        Some(_) => Err(CommandError::bad(
            "count",
            format!(
                "must be from 1 to {}",
                core_document::arrays::MAX_ARRAY_COUNT
            ),
        )),
    }
}

/// An array command's `skip`, when given: occurrence numbers.
fn skip_arg(a: &Args) -> Result<Option<Vec<u32>>, CommandError> {
    let Some(value) = a.0.get("skip").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let items = match value {
        Value::Array(items) => items.as_slice(),
        // An empty Lua table arrives as an object.
        Value::Object(map) if map.is_empty() => &[],
        _ => return Err(CommandError::bad("skip", "is a list of occurrence numbers")),
    };
    items
        .iter()
        .map(|v| {
            v.as_u64()
                .filter(|n| *n >= 1)
                .map(|n| n as u32)
                .ok_or_else(|| CommandError::bad("skip", "holds occurrence numbers from 1"))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// The pattern an array command describes: its `kind` and that kind's
/// values, each taken from `current` (when the kind stays) or its default
/// where it is not given. A path or point array reads its places again.
fn array_pattern(
    document: &core_document::Document,
    registry: &core_document::DocumentService,
    source: BodyId,
    count: u32,
    current: Option<&core_document::ArrayPattern>,
    a: &Args,
) -> Result<core_document::ArrayPattern, CommandError> {
    use core_document::{ArrayPattern, PathSource};
    let vector = |name: &str| -> Result<Option<[f32; 3]>, CommandError> {
        a.0.get(name)
            .filter(|v| !v.is_null())
            .map(|v| numbers_arg(v, name, ["x", "y", "z"]))
            .transpose()
    };
    let kind = match a.opt_string("kind")? {
        Some(kind @ ("linear" | "polar" | "path" | "points")) => kind,
        Some(_) => {
            return Err(CommandError::bad(
                "kind",
                "is \"linear\", \"polar\", \"path\" or \"points\"",
            ));
        }
        None => match current {
            Some(ArrayPattern::Polar { .. }) => "polar",
            Some(ArrayPattern::Path { .. }) => "path",
            Some(ArrayPattern::Points { .. }) => "points",
            _ if a.has("edge") || a.has("align") => "path",
            _ => "linear",
        },
    };
    let spacing = a.opt_number("spacing")?.map(|v| v as f32);
    let angle = a.opt_number("angle")?.map(|v| v as f32);
    let sketch = |a: &Args| -> Result<Option<FeatureId>, CommandError> {
        let Some(id) = a.opt_id("sketch")?.map(FeatureId) else {
            return Ok(None);
        };
        if document.get_feature_meta(id).is_none() {
            return Err(CommandError::bad(
                "sketch",
                "is not a feature of this document",
            ));
        }
        Ok(Some(id))
    };
    let pattern = match kind {
        "polar" => {
            let (center, axis, was) = match current {
                Some(ArrayPattern::Polar {
                    center,
                    axis,
                    angle,
                }) => (*center, *axis, Some(*angle)),
                _ => ([0.0; 3], [0.0, 0.0, 1.0], None),
            };
            ArrayPattern::Polar {
                center: vector("center")?.unwrap_or(center),
                axis: vector("axis")?.unwrap_or(axis),
                angle: angle.or(was).unwrap_or(360.0 / count.max(1) as f32),
            }
        }
        "path" => {
            let was = match current {
                Some(ArrayPattern::Path {
                    along,
                    spacing,
                    align,
                    ..
                }) => Some((*along, *spacing, *align)),
                _ => None,
            };
            let along = match (sketch(a)?, a.0.get("edge").filter(|v| !v.is_null())) {
                (Some(_), Some(_)) => {
                    return Err(CommandError::bad("edge", "goes with no sketch: one path"));
                }
                (Some(feature), None) => {
                    let chain = match a.opt_number("chain")? {
                        Some(n) if n >= 1.0 && n.fract() == 0.0 => n as usize - 1,
                        Some(_) => return Err(CommandError::bad("chain", "counts from 1")),
                        None => 0,
                    };
                    PathSource::Sketch { feature, chain }
                }
                (None, Some(edge)) => edge_arg(document, edge)?,
                (None, None) => was.map(|w| w.0).ok_or_else(|| {
                    CommandError::bad("sketch", "or an edge is needed for a path")
                })?,
            };
            let spacing = spacing.or(was.and_then(|w| w.1));
            let align = a.opt_bool("align")?.or(was.map(|w| w.2)).unwrap_or(false);
            crate::app::array_paths::path_pattern(document, registry, along, count, spacing, align)
                .map_err(|e| CommandError::bad("count", e))?
        }
        "points" => {
            let from = match (sketch(a)?, current) {
                (Some(f), _) => f,
                (None, Some(ArrayPattern::Points { from, .. })) => *from,
                (None, _) => return Err(CommandError::bad("sketch", "is needed for points")),
            };
            crate::app::array_paths::points_pattern(document, registry, from)
                .map_err(|e| CommandError::bad("sketch", e))?
        }
        _ => {
            let (direction, was) = match current {
                Some(ArrayPattern::Linear { direction, spacing }) => (*direction, Some(*spacing)),
                _ => ([1.0, 0.0, 0.0], None),
            };
            let direction = vector("direction")?.unwrap_or(direction);
            ArrayPattern::Linear {
                direction,
                spacing: spacing
                    .or(was)
                    .unwrap_or_else(|| document.spacing_clear_of(source, direction)),
            }
        }
    };
    if !pattern.is_valid() {
        let name = if kind == "polar" { "axis" } else { "direction" };
        return Err(CommandError::bad(name, "has no length"));
    }
    Ok(pattern)
}

/// An edge a path array runs along, given as `{body, point = {x, y, z}}`
/// with the point where the body sits (as `doc.edges` gives it).
fn edge_arg(
    document: &core_document::Document,
    edge: &Value,
) -> Result<core_document::PathSource, CommandError> {
    let body = edge
        .get("body")
        .and_then(Value::as_str)
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(BodyId)
        .filter(|b| document.bodies().iter().any(|e| e.id == *b))
        .ok_or_else(|| CommandError::bad("edge", "names a body of this document as `body`"))?;
    let point = edge
        .get("point")
        .ok_or_else(|| CommandError::bad("edge", "gives a `point` on the edge"))
        .and_then(|p| numbers_arg(p, "edge", ["x", "y", "z"]))?;
    Ok(core_document::PathSource::Edge {
        body,
        point: document.body_placement(body).inverse().point(point),
    })
}

/// How many occurrences an array command makes: the count given, else
/// every point a point array has; a row, a ring or a path needs one.
fn array_count(
    given: Option<u32>,
    pattern: &core_document::ArrayPattern,
) -> Result<u32, CommandError> {
    match (given, pattern) {
        (Some(count), _) => Ok(count),
        (None, core_document::ArrayPattern::Points { points, .. }) => Ok(points.len() as u32),
        (None, _) => Err(CommandError::bad("count", "is needed")),
    }
}

/// Why the document refused an array edit, as the argument it was about.
fn array_refusal(error: &core_document::ArrayError) -> CommandError {
    use core_document::ArrayError;
    let name = match error {
        ArrayError::NoSuchBody | ArrayError::SourceIsInstance => "body",
        ArrayError::NoSuchArray => "array",
        ArrayError::BadCount
        | ArrayError::PathTooShort { .. }
        | ArrayError::TooFewPlaces { .. } => "count",
        ArrayError::BadPattern => "direction",
        ArrayError::EmptyComponent => "component",
    };
    CommandError::bad(name, error.to_string())
}

/// An array as the array commands answer: its id and its copies' bodies.
fn array_answer(document: &core_document::Document, id: core_document::ArrayId) -> Value {
    let Some(array) = document.copy_array(id) else {
        return json!({"id": id.0.to_string(), "instances": []});
    };
    let instances: Vec<String> = array
        .instances
        .iter()
        .map(|i| i.body.0.to_string())
        .collect();
    let mut answer = json!({"id": id.0.to_string(), "instances": instances});
    if array.component.is_some() {
        let groups: Vec<Value> = array
            .instances
            .iter()
            .map(|i| json!(i.group.map(|g| g.0.to_string())))
            .collect();
        answer["components"] = json!(groups);
    }
    answer
}

/// The note a command names, by `id` or by `number`.
fn note_arg(
    document: &core_document::Document,
    a: &Args,
) -> Result<core_document::NoteId, CommandError> {
    if let Some(id) = a.opt_id("id")? {
        return Ok(core_document::NoteId(id));
    }
    let number = a
        .opt_number("number")?
        .ok_or_else(|| CommandError::bad("id", "give the note's id or its number"))?;
    let found = (number >= 1.0 && number.fract() == 0.0 && number <= f64::from(u32::MAX))
        .then(|| document.note_by_number(number as u32))
        .flatten();
    found
        .map(|(_, note)| note.id)
        .ok_or_else(|| CommandError::bad("number", "is not a note of this document"))
}

/// Refuse an agent's call that would `verb` (delete, reword) a note the
/// user wrote.
pub(crate) fn user_note_guard(
    document: &core_document::Document,
    item: TreeItemId,
    a: &Args,
    verb: &str,
) -> Result<(), CommandError> {
    let TreeItemId::Note(id) = item else {
        return Ok(());
    };
    let users = document
        .note(id)
        .is_some_and(|(_, n)| n.author == core_document::NoteAuthor::User);
    if users && note_author(a)? == core_document::NoteAuthor::Agent {
        return Err(CommandError::failed(format!(
            "a user's note is the user's: an agent may resolve or answer it, not {verb} it"
        )));
    }
    Ok(())
}

/// Who a note command acts for: the user unless `author` says the agent.
fn note_author(a: &Args) -> Result<core_document::NoteAuthor, CommandError> {
    match a.opt_string("author")? {
        None | Some("user") => Ok(core_document::NoteAuthor::User),
        Some("agent") => Ok(core_document::NoteAuthor::Agent),
        Some(_) => Err(CommandError::bad("author", "must be user or agent")),
    }
}

fn body_arg(document: &core_document::Document, a: &Args) -> Result<BodyId, CommandError> {
    let body = BodyId(a.id("body")?);
    if document.bodies().iter().any(|b| b.id == body) {
        Ok(body)
    } else {
        Err(CommandError::bad("body", "is not a body of this document"))
    }
}

/// The names of a feature's fields that hold no value, which a script's
/// table cannot show (a nil is no entry): at the top of the data or inside
/// its one variant, where a feature kept as an enum holds its fields.
fn unset_fields(data: &Value) -> Vec<String> {
    let Value::Object(top) = data else {
        return Vec::new();
    };
    let fields = match top.values().next() {
        Some(Value::Object(inner)) if top.len() == 1 => inner,
        _ => top,
    };
    fields
        .iter()
        .filter(|(_, v)| v.is_null())
        .map(|(k, _)| k.clone())
        .collect()
}

/// What kind of feature `node` is, as its workbench names it.
pub(crate) fn kind_of(
    registry: &core_document::DocumentService,
    node: &core_document::FeatureNode,
) -> String {
    registry
        .feature_info(node)
        .map(|info| info.kind_label)
        .unwrap_or_else(|| node.workbench_id.as_str().to_string())
}

/// The tree row an id names: a body, a feature, an imported part, a
/// component or a note.
pub(crate) fn tree_item(
    document: &core_document::Document,
    id: Uuid,
) -> Result<TreeItemId, CommandError> {
    if document.bodies().iter().any(|b| b.id.0 == id) {
        Ok(TreeItemId::Body(BodyId(id)))
    } else if document.get_feature_meta(FeatureId(id)).is_some() {
        Ok(TreeItemId::Feature(FeatureId(id)))
    } else if document.imported_object(id).is_some() {
        Ok(TreeItemId::ImportedObject(id))
    } else if document.component(core_document::ComponentId(id)).is_some() {
        Ok(TreeItemId::Component(core_document::ComponentId(id)))
    } else if document.copy_array(core_document::ArrayId(id)).is_some() {
        Ok(TreeItemId::Array(core_document::ArrayId(id)))
    } else if document.note(core_document::NoteId(id)).is_some() {
        Ok(TreeItemId::Note(core_document::NoteId(id)))
    } else {
        Err(CommandError::bad("id", "is not in this document"))
    }
}

/// The format an export names, else the one its path's extension says.
pub(crate) fn export_format(
    name: Option<&str>,
    path: &std::path::Path,
) -> Result<kernel_ogeom::export::ExportFormat, CommandError> {
    use kernel_ogeom::export::ExportFormat;
    match name {
        Some(name) => match name.to_ascii_lowercase().as_str() {
            "step" | "stp" => Ok(ExportFormat::Step),
            "step_nurbs" => Ok(ExportFormat::StepNurbs),
            "stl" => Ok(ExportFormat::Stl),
            "3mf" => Ok(ExportFormat::ThreeMf),
            _ => Err(CommandError::bad(
                "format",
                "must be step, step_nurbs, stl or 3mf",
            )),
        },
        None => ExportFormat::of_path(path).ok_or_else(|| {
            CommandError::bad(
                "format",
                "is needed when the path has no .step, .stl or .3mf",
            )
        }),
    }
}

/// A list of body ids, when one is given.
pub(crate) fn body_list(value: Option<&Value>) -> Result<Option<Vec<BodyId>>, CommandError> {
    match value {
        Some(Value::Array(list)) => list
            .iter()
            .map(|v| {
                v.as_str()
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .map(BodyId)
                    .ok_or_else(|| CommandError::bad("bodies", "must be a list of body ids"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        _ => Ok(None),
    }
}

/// The features of `body` (or all of them) in build order.
/// What `doc.rebuild` answers: every feature that failed, in history
/// order, as `{feature, name, error}`: its build error, or the formula of
/// its that fails.
pub(crate) fn rebuild_failures(document: &core_document::Document) -> Vec<Value> {
    features_in_order(document, None)
        .into_iter()
        .filter_map(|n| {
            let error = n.error.clone().or_else(|| document.formula_error(n.id))?;
            Some(json!({"feature": n.id.0.to_string(), "name": n.name, "error": error}))
        })
        .collect()
}

pub(crate) fn features_in_order(
    document: &core_document::Document,
    body: Option<BodyId>,
) -> Vec<&core_document::FeatureNode> {
    let mut nodes: Vec<_> = document
        .feature_tree()
        .all_nodes()
        .map(|(_, n)| n)
        .filter(|n| body.is_none() || n.body == body)
        .collect();
    nodes.sort_by_key(|n| n.seq);
    nodes
}

/// Each face of a mesh: what surface it is, a point on it, its area, and a
/// flat face's normal or a turned face's axis.
pub(super) fn faces_of(mesh: &kernel_api::TriMesh) -> Value {
    let count = mesh.face_surfaces.len().max(
        mesh.faces
            .iter()
            .map(|f| *f as usize + 1)
            .max()
            .unwrap_or(0),
    );
    // Per face: area, the area-weighted centre, and the triangle centre
    // nearest it (a point that is on the face even when it curves).
    let mut area = vec![0.0f32; count];
    let mut centre = vec![[0.0f32; 3]; count];
    let tri = |t: usize| {
        let p = |i: usize| glam::Vec3::from(mesh.positions[mesh.indices[3 * t + i] as usize]);
        (p(0), p(1), p(2))
    };
    for (t, face) in mesh.faces.iter().enumerate() {
        let (a, b, c) = tri(t);
        let w = (b - a).cross(c - a).length() / 2.0;
        let f = *face as usize;
        area[f] += w;
        let m = (a + b + c) / 3.0;
        for i in 0..3 {
            centre[f][i] += m[i] * w;
        }
    }
    let mut on_face = vec![None::<(f32, glam::Vec3)>; count];
    for (t, face) in mesh.faces.iter().enumerate() {
        let f = *face as usize;
        if area[f] <= 0.0 {
            continue;
        }
        let target = glam::Vec3::from(centre[f]) / area[f];
        let (a, b, c) = tri(t);
        let m = (a + b + c) / 3.0;
        let d = m.distance(target);
        if on_face[f].is_none_or(|(best, _)| d < best) {
            on_face[f] = Some((d, m));
        }
    }
    let list = (0..count)
        .filter_map(|f| {
            let (_, point) = on_face[f]?;
            let surface = mesh.face_surfaces.get(f).copied().unwrap_or_default();
            let mut out = surface_json(surface);
            out["index"] = json!(f);
            out["point"] = json!(point.to_array());
            out["area"] = json!(area[f]);
            // What a rebuild finds the face by, where it has a name: a
            // string, since a script's numbers cannot hold every name.
            if let Some(name) = mesh.face_names.get(f).filter(|n| **n != 0) {
                out["name"] = json!(name.to_string());
            }
            Some(out)
        })
        .collect();
    Value::Array(list)
}

/// A face's surface as `doc.faces` gives it: its kind and what places
/// and sizes it.
pub(crate) fn surface_json(surface: kernel_api::FaceSurface) -> Value {
    let kind = match surface {
        kernel_api::FaceSurface::Plane { .. } => "plane",
        kernel_api::FaceSurface::Cylinder { .. } => "cylinder",
        kernel_api::FaceSurface::Cone { .. } => "cone",
        kernel_api::FaceSurface::Sphere { .. } => "sphere",
        kernel_api::FaceSurface::Torus { .. } => "torus",
        kernel_api::FaceSurface::Other => "other",
    };
    let mut out = json!({"kind": kind});
    match surface {
        kernel_api::FaceSurface::Plane { normal, .. } => out["normal"] = json!(normal),
        kernel_api::FaceSurface::Cylinder { radius, .. } => out["radius"] = json!(radius),
        kernel_api::FaceSurface::Sphere { radius, center } => {
            out["radius"] = json!(radius);
            out["centre"] = json!(center);
        }
        kernel_api::FaceSurface::Cone { half_angle, .. } => {
            out["half_angle"] = json!(half_angle.to_degrees());
        }
        kernel_api::FaceSurface::Torus {
            major_radius,
            minor_radius,
            ..
        } => {
            out["major_radius"] = json!(major_radius);
            out["minor_radius"] = json!(minor_radius);
        }
        _ => {}
    }
    if let Some((p, d)) = surface.axis() {
        out["axis"] = json!({"point": p, "direction": d});
    }
    out
}

/// What a deletion took away, for the caller's own bookkeeping.
pub(crate) struct Deleted {
    /// The undo entry's label.
    pub label: String,
    /// The bodies that went, with everything on them.
    pub bodies: Vec<BodyId>,
    /// What the owning bench's delete hook asked of the host, when one ran.
    pub outcome: Option<core_document::HookOutcome>,
}

/// Delete what a tree item stands for, as `doc.delete` does with or
/// without a window: a feature through the bench claiming its kind (one of
/// a package that is not running is refused, its data waiting for the
/// package), then its body rebuilt from what is left; a body with every
/// feature on it; an imported row with its subtree and their bodies; a
/// component taken apart; a note, a body's notes or an array of copies.
/// `params` describes the view the bench's hook sees.
pub(crate) fn delete_item(
    document: &mut core_document::Document,
    registry: &mut core_document::DocumentService,
    item: TreeItemId,
    params: crate::app::workbench_host::WbCtxParams,
) -> Result<Deleted, CommandError> {
    let deleted = |label: &str, bodies: Vec<BodyId>| Deleted {
        label: label.to_string(),
        bodies,
        outcome: None,
    };
    match item {
        TreeItemId::Feature(feature) => delete_feature(document, registry, feature, params),
        TreeItemId::Body(body) => {
            if document.remove_body(body) {
                Ok(deleted("Delete body", vec![body]))
            } else {
                Err(CommandError::failed("the body could not be deleted"))
            }
        }
        TreeItemId::ImportedObject(node) => {
            let bodies = remove_imported_rows(document, node);
            Ok(deleted("Delete body", bodies))
        }
        TreeItemId::Component(component) => {
            let name = document
                .component(component)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            document.remove_component(component).map_err(|err| {
                CommandError::failed(format!("could not take {name} apart: {err}"))
            })?;
            Ok(deleted(&format!("Take apart {name}"), Vec::new()))
        }
        TreeItemId::Note(note) => {
            if document.remove_note(note) {
                Ok(deleted("Delete note", Vec::new()))
            } else {
                Err(CommandError::failed("the note could not be deleted"))
            }
        }
        TreeItemId::Notes(body) => {
            document.set_body_notes(body, Vec::new());
            Ok(deleted("Delete notes", Vec::new()))
        }
        // The array goes with every copy it owns; its source stays.
        TreeItemId::Array(array) => {
            let Some(found) = document.copy_array(array) else {
                return Err(CommandError::bad("id", "is not in this document"));
            };
            let name = found.name.clone();
            let bodies: Vec<BodyId> = found.copy_bodies().collect();
            if document.remove_copy_array(array) {
                Ok(deleted(&format!("Delete {name}"), bodies))
            } else {
                Err(CommandError::failed(format!("{name} could not be deleted")))
            }
        }
        TreeItemId::DocumentRoot => Err(CommandError::bad("id", "cannot be deleted")),
    }
}

/// Delete a feature the way the bench claiming its kind deletes it, then
/// rebuild its body from what is left (or drop the shape nothing builds
/// any more). A kind no bench claims is removed, unless a package that is
/// not running made it.
fn delete_feature(
    document: &mut core_document::Document,
    registry: &mut core_document::DocumentService,
    feature: FeatureId,
    params: crate::app::workbench_host::WbCtxParams,
) -> Result<Deleted, CommandError> {
    let node = document
        .get_feature_meta(feature)
        .ok_or_else(|| CommandError::bad("id", "is not in this document"))?;
    let body = node.body;
    let owner = registry.owner_id_of(&node.workbench_id).cloned();
    if owner.is_none()
        && let Some(why) = core_document::FeatureInfo::missing_package(node)
    {
        return Err(CommandError::failed(why));
    }
    let mut outcome = None;
    let removed = match owner {
        Some(owner) => {
            let mut ctx = params.context(document, registry);
            let wb = registry
                .workbench_mut(&owner)
                .map_err(|e| CommandError::failed(e.to_string()))?;
            let removed = wb.delete_feature(&mut ctx, feature);
            outcome = Some(core_document::HookOutcome::take(&mut ctx));
            crate::PrintCadApp::flush_logs(ctx.drain_logs());
            removed
        }
        None => document.remove_feature(feature).is_ok(),
    };
    if !removed {
        return Err(CommandError::failed("the feature could not be deleted"));
    }
    if let Some(body) = body {
        registry.invalidate_body(document, body);
    }
    Ok(Deleted {
        label: "Delete feature".to_string(),
        bodies: Vec::new(),
        outcome,
    })
}

/// Take an imported row and its subtree out of the graph, with the bodies
/// those rows stood for. The bodies that went.
fn remove_imported_rows(document: &mut core_document::Document, node: Uuid) -> Vec<BodyId> {
    let mut doomed: Vec<Uuid> = Vec::new();
    let mut stack = vec![node];
    while let Some(id) = stack.pop() {
        let Some(entry) = document.imported_object(id) else {
            continue;
        };
        stack.extend(entry.children.iter().copied());
        doomed.push(id);
    }
    let bodies: Vec<BodyId> = doomed
        .iter()
        .filter_map(|id| document.imported_object(*id).and_then(|n| n.body_id))
        .collect();
    // What stays, with the deleted rows dropped from their parents'
    // children.
    let roots: Vec<Uuid> = document
        .imported_object_roots()
        .iter()
        .copied()
        .filter(|id| !doomed.contains(id))
        .collect();
    let mut kept = std::collections::HashMap::new();
    let mut stack: Vec<Uuid> = roots.clone();
    while let Some(id) = stack.pop() {
        let Some(entry) = document.imported_object(id) else {
            continue;
        };
        let mut entry = entry.clone();
        entry.children.retain(|child| !doomed.contains(child));
        stack.extend(entry.children.iter().copied());
        kept.insert(id, entry);
    }
    document.set_imported_object_graph(roots, kept);
    let mut bodies = bodies;
    bodies.retain(|body| document.remove_body(*body));
    bodies
}

/// Show or hide what a tree item stands for, as `doc.set_visible` does
/// with or without a window: a body, a feature, an imported row, every
/// body of a component or an array, a note. The bodies it showed or hid.
pub(crate) fn set_item_visible(
    document: &mut core_document::Document,
    item: TreeItemId,
    visible: bool,
) -> Result<Vec<BodyId>, CommandError> {
    let bodies = match item {
        TreeItemId::Body(body) => vec![body],
        TreeItemId::Feature(feature) => {
            document.set_feature_visible(feature, visible);
            return Ok(Vec::new());
        }
        TreeItemId::ImportedObject(node) => {
            document.set_imported_object_visibility(node, visible);
            return Ok(Vec::new());
        }
        TreeItemId::Component(component) => document.component_bodies(component),
        TreeItemId::Array(array) => document
            .copy_array(array)
            .map(|a| a.copy_bodies().collect())
            .unwrap_or_default(),
        TreeItemId::Note(note) => {
            if let Some((_, found)) = document.note(note) {
                let mut changed = found.clone();
                changed.hidden = !visible;
                document.set_note(changed);
            }
            return Ok(Vec::new());
        }
        TreeItemId::DocumentRoot | TreeItemId::Notes(_) => {
            return Err(CommandError::bad("id", "cannot be hidden"));
        }
    };
    for body in &bodies {
        document.set_body_visible(*body, visible);
    }
    Ok(bodies)
}
