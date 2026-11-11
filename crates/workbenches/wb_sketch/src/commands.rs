//! The sketcher's commands: make a sketch and draw in it by numbers, for
//! scripts and other callers that are not a click.
//!
//! Coordinates are the sketch's own, in millimetres. A drawn end that lands
//! exactly on a point the sketch already has takes that point, as a
//! snapped click does, so lines drawn end to end close a profile.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, DatumFeature, DatumShape,
    FeatureId, FileImport, ParamKind, WorkbenchContext, WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::feature::DatumSupport;
use crate::feature::SketchFeature;
use crate::sketch::{GeometryElement, Line, Point, Sketch, SketchPlane, Vec2D};

mod constrain;
mod draw;
mod edit;
mod external;
mod files;
mod images;
mod placement;
mod query;
mod scan;
mod splines;
#[cfg(test)]
mod tests;
mod text;

pub(crate) use constrain::{ConstraintFlags, remove_superseded, set_constraints};
pub(crate) use constrain::{scaled_formula, switch_circle_dimension};
pub(crate) use draw::paste;
pub(crate) use edit::delete_items;
pub(crate) use external::{add_external, set_external_defining};
pub(crate) use files::export;
pub(crate) use images::{calibrate_image, fit_edge};
pub(crate) use placement::{carbon_copy, merge, mirror_sketch};

/// The commands in the order they are registered, which is the order the
/// scripting reference lists them, whichever family each belongs to. A new
/// command goes in here too: `the_order_names_every_command` fails until it does.
const ORDER: &[&str] = &[
    "sketch.new",
    "sketch.import_dxf",
    "sketch.image",
    "sketch.set_image",
    "sketch.point",
    "sketch.line",
    "sketch.polyline",
    "sketch.rect",
    "sketch.circle",
    "sketch.arc",
    "sketch.export_dxf",
    "sketch.export_svg",
    "sketch.export_csv",
    "sketch.align_image",
    "sketch.turn_image",
    "sketch.bolt_circle",
    "sketch.calibrate_axis",
    "sketch.save_scanner_profile",
    "sketch.image_profile",
    "sketch.calibrate_image",
    "sketch.image_objects",
    "sketch.fit_edge",
    "sketch.candidates",
    "sketch.design_values",
    "sketch.geometry",
    "sketch.constrain",
    "sketch.set_value",
    "sketch.draw",
    "sketch.drag",
    "sketch.attachment",
    "sketch.external_from",
    "sketch.external_defining",
    "sketch.solver_settings",
    "sketch.repair",
    "sketch.restore",
    "sketch.set_plane",
    "sketch.array",
    "sketch.text",
    "sketch.text_edit",
    "sketch.explode_text",
    "sketch.to_bspline",
    "sketch.spline_degree",
    "sketch.insert_knot",
    "sketch.knot_multiplicity",
    "sketch.spline_knots",
    "sketch.spline_weight",
    "sketch.join",
    "sketch.set_constraint",
    "sketch.mirror_sketch",
    "sketch.merge",
    "sketch.carbon_copy",
    "sketch.paste",
    "sketch.external",
    "sketch.fit_scan",
    "sketch.intersection",
    "sketch.constraints",
    "sketch.regions",
    "sketch.wall_thickness",
    "sketch.status",
    "sketch.delete",
    "sketch.construction",
    "sketch.internal_geometry",
    "sketch.section_view",
    "sketch.remove_axis_alignment",
];

/// Register every command this module runs.
pub fn register(context: &mut WorkbenchContext) {
    let mut specs = Vec::new();
    placement::configure(&mut specs);
    draw::configure(&mut specs);
    constrain::configure(&mut specs);
    text::configure(&mut specs);
    splines::configure(&mut specs);
    external::configure(&mut specs);
    images::configure(&mut specs);
    scan::configure(&mut specs);
    files::configure(&mut specs);
    query::configure(&mut specs);
    edit::configure(&mut specs);
    specs.sort_by_key(|spec| {
        ORDER
            .iter()
            .position(|id| *id == spec.id)
            .unwrap_or(ORDER.len())
    });
    for spec in specs {
        context.register_command(spec);
    }
    context.register_import(FileImport::new("DXF drawing", ["dxf"], "sketch.import_dxf"));
    context.register_import(FileImport::new(
        "Reference image (sketch)",
        ["png", "jpg", "jpeg"],
        "sketch.image",
    ));
}

/// `spec` taking the sketch it works in.
fn sketch(spec: CommandSpec) -> CommandSpec {
    spec.param("sketch", ParamKind::Id, "The sketch to draw in")
}

/// The arguments that say where a new sketch goes.
fn placing(spec: CommandSpec) -> CommandSpec {
    spec.optional(
        "body",
        ParamKind::Id,
        "The body it belongs to; the selected body, else a new one",
    )
    .optional("plane", ParamKind::String, "XY (the default), XZ or YZ")
    .optional(
        "offset",
        ParamKind::Number,
        "How far along the plane's normal it sits",
    )
    .optional("name", ParamKind::String, "Its name in the tree")
    .optional(
        "attachment",
        ParamKind::Any,
        "Attached as a datum plane is, the attachment as a datum keeps it (doc.feature \
         on a datum shows it): {Face = {face = {point = {x, y, z}, normal = {x, y, z}}}}, \
         {ThreePoints = {points = {{At = {point = {x, y, z}}}, ...}}} and the like, faces \
         and edges on the body's own solid; the sketch follows what it stands on. \
         design.datum makes the same from plainer arguments, and `on` takes that datum",
    )
    .optional(
        "attachment_offset",
        ParamKind::Any,
        "The attachment's offset, as a datum's",
    )
    .optional(
        "on",
        ParamKind::Id,
        "A datum plane, or a coordinate system whose XY, XZ or YZ plane (see plane) it takes",
    )
    .optional(
        "normal",
        ParamKind::List,
        "A plane of its own instead: its normal as {x, y, z}",
    )
    .optional(
        "origin",
        ParamKind::List,
        "With normal: where the plane's origin sits, {x, y, z}",
    )
    .optional(
        "x_axis",
        ParamKind::List,
        "With normal: the sketch's X direction, {x, y, z}",
    )
}

/// Run command `id`, or say it is not one of these.
pub fn run(id: &str, args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    if placement::owns(id) {
        return placement::run(id, args, ctx);
    }
    if draw::owns(id) {
        return draw::run(id, args, ctx);
    }
    if constrain::owns(id) {
        return constrain::run(id, args, ctx);
    }
    if text::owns(id) {
        return text::run(id, args, ctx);
    }
    if splines::owns(id) {
        return splines::run(id, args, ctx);
    }
    if external::owns(id) {
        return external::run(id, args, ctx);
    }
    if images::owns(id) {
        return images::run(id, args, ctx);
    }
    if scan::owns(id) {
        return scan::run(id, args, ctx);
    }
    if files::owns(id) {
        return files::run(id, args, ctx);
    }
    if query::owns(id) {
        return query::run(id, args, ctx);
    }
    if edit::owns(id) {
        return edit::run(id, args, ctx);
    }
    Err(CommandError::Unknown(id.to_string()))
}

/// Add `sketch` as a new sketch feature where `a` places it: on a base
/// plane, a datum or a plane of its own, in the body given or selected,
/// else a new one.
fn add_sketch(
    a: &Args,
    ctx: &mut WorkbenchRuntimeContext,
    mut sketch: Sketch,
    made_by: Option<crate::generator::Generator>,
) -> Result<FeatureId, CommandError> {
    let mut plane = match a.opt_string("plane")?.unwrap_or("XY") {
        p if p.eq_ignore_ascii_case("XY") => SketchPlane::xy(),
        p if p.eq_ignore_ascii_case("XZ") => SketchPlane::xz(),
        p if p.eq_ignore_ascii_case("YZ") => SketchPlane::yz(),
        _ => return Err(CommandError::bad("plane", "must be XY, XZ or YZ")),
    };
    if a.has("normal") {
        plane = custom_plane(a)?;
    }
    let mut datum_body = None;
    let offset = a.opt_number("offset")?.unwrap_or(0.0);
    let mut support = None;
    if let Some(on) = a.opt_id("on")? {
        let (on_datum, body) = datum_support(ctx, FeatureId(on), a.opt_string("plane")?, offset)?;
        plane = on_datum.1;
        support = Some(on_datum.0);
        datum_body = body;
    } else {
        for (o, n) in plane.origin.iter_mut().zip(plane.normal) {
            *o += n * offset;
        }
    }
    let body = match a.opt_id("body")?.or(datum_body.map(|b| b.0)) {
        Some(id) => {
            let body = BodyId(id);
            if ctx.document.body(body).is_none() {
                return Err(CommandError::bad(
                    "body",
                    core_document::command::NOT_A_BODY,
                ));
            }
            body
        }
        None => match ctx.selected_body_id {
            Some(id) => BodyId(id),
            None => ctx.document.create_body(None),
        },
    };
    // Attached by a mode: the plane is where the attachment puts it on
    // the body, and follows it.
    let attached = match args_value(a, "attachment") {
        Some(value) => {
            let mut value = value.clone();
            following(&mut value);
            let attachment: core_document::DatumAttachment = serde_json::from_value(value)
                .map_err(|e| CommandError::bad("attachment", e.to_string()))?;
            let offset = match args_value(a, "attachment_offset") {
                Some(v) => serde_json::from_value(v.clone())
                    .map_err(|e| CommandError::bad("attachment_offset", e.to_string()))?,
                None => core_document::AttachmentOffset::default(),
            };
            let attached = settle_attached(ctx, body, attachment, offset)
                .map_err(|e| CommandError::bad("attachment", e))?;
            plane = attached.plane();
            Some(attached)
        }
        None => None,
    };
    sketch.plane = plane;
    let name = sketch.name.clone();
    let mut feature = SketchFeature::new(sketch, plane);
    feature.support = support;
    feature.attached = attached;
    if let Some(made_by) = made_by {
        feature.generator = Some(made_by);
        crate::generator::regenerate(&mut feature).map_err(CommandError::failed)?;
    }
    ctx.document
        .add_feature_in_body(feature, name, Some(body))
        .map_err(|e| CommandError::failed(e.to_string()))
}

/// Every face and edge an attachment given to a command names is on the
/// sketch's own body, so the sketch follows it, unless the script says
/// otherwise: `follows` set on each anchor (a point with a normal or a
/// direction) that does not give it.
fn following(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let anchor = map.contains_key("point")
                && (map.contains_key("normal") || map.contains_key("direction"));
            if anchor && !map.contains_key("follows") {
                map.insert("follows".into(), Value::Bool(true));
            }
            for child in map.values_mut() {
                following(child);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(following),
        _ => {}
    }
}

/// An argument's raw value, when given and not nil.
fn args_value<'a>(a: &'a Args, name: &str) -> Option<&'a Value> {
    a.0.get(name).filter(|v| !v.is_null())
}

/// An attachment on `body`, filled in from its solid where there is one,
/// as a sketch keeps it.
pub(crate) fn settle_attached(
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    attachment: core_document::DatumAttachment,
    offset: core_document::AttachmentOffset,
) -> Result<crate::feature::AttachedSupport, String> {
    let mut datum = crate::feature::AttachedSupport { attachment, offset }.datum();
    core_document::attach::settle(ctx, body, &mut datum)?;
    Ok(crate::feature::AttachedSupport {
        attachment: datum.attachment,
        offset: datum.offset,
    })
}

fn load(ctx: &WorkbenchRuntimeContext, id: FeatureId) -> Result<SketchFeature, CommandError> {
    let data = ctx
        .document
        .get_feature_data(id)
        .ok_or_else(|| CommandError::bad("sketch", "is not a feature of this document"))?;
    SketchFeature::from_json(data)
        .map_err(|why| CommandError::bad("sketch", format!("is not a sketch: {why}")))
}

fn vec(x: f64, y: f64) -> Vec2D {
    Vec2D::new(x, y)
}

fn positive(a: &Args, name: &str) -> Result<f64, CommandError> {
    let value = a.number(name)?;
    if value > 0.0 {
        Ok(value)
    } else {
        Err(CommandError::bad(name, "must be more than zero"))
    }
}

/// The point at `at`: one the sketch has there already, else a new one.
fn point_at(sketch: &mut Sketch, at: Vec2D) -> Uuid {
    let existing = sketch.geometry.iter().find_map(|g| match g {
        GeometryElement::Point(p) if (p.position - at).to_glam().length() < 1e-5 => Some(p.id),
        _ => None,
    });
    existing.unwrap_or_else(|| sketch.add_geometry(GeometryElement::Point(Point::new(at))))
}

fn line(sketch: &mut Sketch, from: Vec2D, to: Vec2D) -> Result<Uuid, CommandError> {
    if (to - from).to_glam().length() < 1e-5 {
        return Err(CommandError::failed("a line needs two different ends"));
    }
    let (a, b) = (point_at(sketch, from), point_at(sketch, to));
    Ok(sketch.add_geometry(GeometryElement::Line(Line::new(a, b))))
}

fn polyline(
    sketch: &mut Sketch,
    points: &[Vec2D],
    closed: bool,
) -> Result<Vec<String>, CommandError> {
    if points.len() < 2 {
        return Err(CommandError::bad("points", "needs at least two points"));
    }
    let mut segments: Vec<(Vec2D, Vec2D)> = points.windows(2).map(|p| (p[0], p[1])).collect();
    if closed && points.len() > 2 {
        segments.push((points[points.len() - 1], points[0]));
    }
    let mut ids = Vec::new();
    for (from, to) in segments {
        let id = line(sketch, from, to)?;
        // Level or upright as given, it stays so, as the line tool holds it.
        if (to.y - from.y).abs() <= 1e-9 && (to.x - from.x).abs() > 1e-9 {
            sketch.add_constraint(crate::sketch::ConstraintKind::Horizontal { element: id });
        } else if (to.x - from.x).abs() <= 1e-9 && (to.y - from.y).abs() > 1e-9 {
            sketch.add_constraint(crate::sketch::ConstraintKind::Vertical { element: id });
        }
        ids.push(id.to_string());
    }
    Ok(ids)
}

/// Where a sketch on a datum stands, and the plane that puts it there now.
type OnDatum = (DatumSupport, SketchPlane);

/// A sketch's place on datum `id`: a datum plane, or one of a coordinate
/// system's three (`which`, XY when left out), `offset` along its normal;
/// with the plane that puts it on now and the body the datum is in.
fn datum_support(
    ctx: &WorkbenchRuntimeContext,
    id: FeatureId,
    which: Option<&str>,
    offset: f64,
) -> Result<(OnDatum, Option<BodyId>), CommandError> {
    let not_a_plane = || CommandError::bad("on", "is not a datum plane or coordinate system");
    let node = ctx.document.get_feature_meta(id).ok_or_else(not_a_plane)?;
    let datum = DatumFeature::from_json(&node.data).map_err(|why| {
        CommandError::bad(
            "on",
            format!("is not a datum plane or coordinate system: {why}"),
        )
    })?;
    let plane = match datum.shape {
        DatumShape::Plane { .. } => None,
        DatumShape::CoordinateSystem { .. } => {
            let which = which.unwrap_or("XY");
            if !["XY", "XZ", "YZ"]
                .iter()
                .any(|p| p.eq_ignore_ascii_case(which))
            {
                return Err(CommandError::bad("plane", "must be XY, XZ or YZ"));
            }
            Some(which.to_ascii_uppercase())
        }
        _ => return Err(not_a_plane()),
    };
    let support = DatumSupport {
        datum: id,
        plane,
        offset,
        shift: [0.0, 0.0],
        turn: 0.0,
    };
    let values = ctx
        .document
        .feature_values(id)
        .cloned()
        .unwrap_or_else(|| node.data.clone());
    let at = support.plane_from(&values).ok_or_else(not_a_plane)?;
    Ok(((support, at), node.body))
}

/// A plane from `normal`, `origin` and `x_axis`, the last two optional.
fn custom_plane(a: &Args) -> Result<SketchPlane, CommandError> {
    let normal = unit(a.vec3("normal")?, "normal")?;
    let origin = a.opt_vec3("origin")?.unwrap_or([0.0; 3]);
    // Any direction square to the normal serves when none is given.
    let guess = if normal[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let wanted = a
        .opt_vec3("x_axis")?
        .unwrap_or_else(|| cross(guess, normal));
    let along = dot(wanted, normal);
    let x_axis = unit(
        [
            wanted[0] - along * normal[0],
            wanted[1] - along * normal[1],
            wanted[2] - along * normal[2],
        ],
        "x_axis",
    )?;
    let y_axis = cross(normal, x_axis);
    let f = |v: [f64; 3]| v.map(|c| c);
    Ok(SketchPlane {
        origin: f(origin),
        normal: f(normal),
        x_axis: f(x_axis),
        y_axis: f(y_axis),
    })
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(v: [f64; 3], name: &str) -> Result<[f64; 3], CommandError> {
    let length = dot(v, v).sqrt();
    if length < 1e-9 {
        return Err(CommandError::bad(
            name,
            "must not be zero, or along the normal",
        ));
    }
    Ok(v.map(|c| c / length))
}

/// Ids named in a list: element and constraint ids, and the names of the
/// sketch's origin and axes.
fn ids(value: Option<&Value>, name: &str, sketch: &Sketch) -> Result<Vec<Uuid>, CommandError> {
    let list = value
        .and_then(Value::as_array)
        .ok_or_else(|| CommandError::bad(name, "must be a list of ids"))?;
    list.iter()
        .map(|v| {
            let text = v
                .as_str()
                .ok_or_else(|| CommandError::bad(name, "must be a list of ids"))?;
            let id = match text {
                "origin" => crate::sketch::ORIGIN_ID,
                "x_axis" => crate::sketch::X_AXIS_ID,
                "y_axis" => crate::sketch::Y_AXIS_ID,
                other => Uuid::parse_str(other)
                    .map_err(|_| CommandError::bad(name, format!("has `{other}`, not an id")))?,
            };
            let known = sketch.get_geometry(id).is_some()
                || sketch.constraints.iter().any(|c| c.id == id)
                || [
                    crate::sketch::ORIGIN_ID,
                    crate::sketch::X_AXIS_ID,
                    crate::sketch::Y_AXIS_ID,
                ]
                .contains(&id);
            if known {
                Ok(id)
            } else {
                Err(CommandError::bad(
                    name,
                    format!("has {id}, which is not in this sketch"),
                ))
            }
        })
        .collect()
}

/// Store an edited sketch and answer `answer`.
fn save(
    ctx: &mut WorkbenchRuntimeContext,
    id: FeatureId,
    mut feature: SketchFeature,
    answer: Value,
) -> CommandResult {
    crate::solver::solve(&mut feature.sketch);
    ctx.document
        .update_feature_data(id, feature.to_json())
        .map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document.mark_feature_dirty(id);
    Ok(answer)
}

pub(crate) fn ids_of(
    sketch: &Sketch,
) -> (
    std::collections::HashSet<Uuid>,
    std::collections::HashSet<Uuid>,
) {
    (
        sketch.geometry.iter().map(GeometryElement::id).collect(),
        sketch.constraints.iter().map(|c| c.id).collect(),
    )
}

/// `{elements, constraints}`: what the sketch gained since `before`.
pub(crate) fn made_since(
    sketch: &Sketch,
    before: &(
        std::collections::HashSet<Uuid>,
        std::collections::HashSet<Uuid>,
    ),
) -> Value {
    let elements: Vec<String> = sketch
        .geometry
        .iter()
        .map(GeometryElement::id)
        .filter(|id| !before.0.contains(id))
        .map(|id| id.to_string())
        .collect();
    let constraints: Vec<String> = sketch
        .constraints
        .iter()
        .map(|c| c.id)
        .filter(|id| !before.1.contains(id))
        .map(|id| id.to_string())
        .collect();
    json!({"elements": elements, "constraints": constraints})
}

/// Sketch `id` with its plane where its body sits.
fn placed(document: &core_document::Document, id: FeatureId) -> Option<SketchFeature> {
    let mut feature = crate::stored_sketch(document, id)?;
    let placement = crate::sketch_placement(document, id);
    feature.plane = crate::placed_plane(&feature.plane, &placement);
    Some(feature)
}

/// Named arguments from a JSON object.
pub(crate) fn args(value: Value) -> CommandArgs {
    match value {
        Value::Object(map) => map,
        _ => CommandArgs::new(),
    }
}

/// `sketch.draw`: the tool run over the points as clicks, the same step a
/// click in the viewport takes. The ids of what it made.
fn draw(
    sketch: &mut Sketch,
    a: &Args,
    args: &CommandArgs,
) -> Result<(Vec<String>, Vec<String>), CommandError> {
    use crate::step;
    let name = a.string("tool")?;
    let tool = if name.starts_with("sketch.") {
        name.to_string()
    } else {
        format!("sketch.{name}")
    };
    if !DRAW_TOOLS.contains(&tool.trim_start_matches("sketch.")) {
        return Err(CommandError::bad("tool", format!("has no tool `{name}`")));
    }
    let settings = step::StepSettings {
        tol: a.opt_number("tolerance")?.unwrap_or(1e-3),
        params: step::params_from_json(args.get("params"))
            .map_err(|e| CommandError::bad("params", e))?,
        construction: a.opt_bool("construction")?.unwrap_or(false),
        avoid_redundant: a.opt_bool("avoid_redundant")?.unwrap_or(true),
    };
    let selected: std::collections::HashSet<Uuid> = match args.get("selection") {
        Some(v) if !v.is_null() => ids(Some(v), "selection", sketch)?.into_iter().collect(),
        _ => Default::default(),
    };
    let events = args
        .get("points")
        .and_then(Value::as_array)
        .ok_or_else(|| CommandError::bad("points", "must be a list of clicks"))?;
    let elements_before: std::collections::HashSet<Uuid> =
        sketch.geometry.iter().map(GeometryElement::id).collect();
    let constraints_before: std::collections::HashSet<Uuid> =
        sketch.constraints.iter().map(|c| c.id).collect();
    let mut state = crate::tools::ToolState::Idle;
    let mut capture = crate::ovp::DimCapture::default();
    for event in events {
        match event {
            Value::String(word) => match word.as_str() {
                w if crate::tools::PolySegment::of_word(w).is_some() => {
                    if let Some(segment) = crate::tools::PolySegment::of_word(w) {
                        crate::tools::set_polyline_segment(&mut state, segment);
                    }
                }
                "finish" => {
                    let effect =
                        crate::tools::finish_click_sequence(&mut state, sketch, &settings.params);
                    if effect.changed {
                        crate::solver::solve(sketch);
                    }
                }
                other => {
                    return Err(CommandError::bad(
                        "points",
                        format!(
                            "has `{other}`; a word there is line, arc, perpendicular_arc, \
                             reverse_arc or finish"
                        ),
                    ));
                }
            },
            // A line tool's arc: tangent to what ends where it draws from.
            click
                if tool == "sketch.line"
                    && click.get("arc").and_then(Value::as_bool) == Some(true) =>
            {
                let (at, typed, constrain) = click_of(click)?;
                let outcome = step::line_arc_click(
                    &mut state,
                    &mut capture,
                    sketch,
                    at,
                    &typed,
                    constrain,
                    &settings,
                    &selected,
                );
                if outcome.changed {
                    crate::solver::solve(sketch);
                }
            }
            click => {
                let (at, typed, constrain) = click_of(click)?;
                let outcome = step::click(
                    &mut state,
                    &mut capture,
                    &tool,
                    sketch,
                    at,
                    &typed,
                    constrain,
                    &settings,
                    &selected,
                );
                // The sketch settles after every click that changes it, as
                // it does between clicks in the viewport, so the next click
                // meets the same sketch.
                if outcome.changed || outcome.added > 0 {
                    crate::solver::solve(sketch);
                }
            }
        }
    }
    let elements = sketch
        .geometry
        .iter()
        .map(GeometryElement::id)
        .filter(|id| !elements_before.contains(id))
        .map(|id| id.to_string())
        .collect();
    let constraints = sketch
        .constraints
        .iter()
        .map(|c| c.id)
        .filter(|id| !constraints_before.contains(id))
        .map(|id| id.to_string())
        .collect();
    Ok((elements, constraints))
}

/// The tools `sketch.draw` runs, without the `sketch.` prefix.
const DRAW_TOOLS: &[&str] = &[
    "point",
    "line",
    "polyline",
    "rect",
    "rect_rounded",
    "rect_center",
    "rect3",
    "rect_center3",
    "rect_frame",
    "circle",
    "circle3",
    "arc",
    "arc3",
    "ellipse",
    "ellipse3",
    "ellipse_arc",
    "parabola",
    "hyperbola",
    "bspline",
    "polygon",
    "slot",
    "arc_slot",
    "fillet",
    "chamfer",
    "trim",
    "extend",
    "split",
    "bspline_knot",
    "offset",
    "translate",
    "rotate",
    "scale",
    "mirror",
];

/// One click of `sketch.draw`: where, the values typed at it, and whether
/// they become constraints.
type Click = (Vec2D, Vec<(crate::ovp::FieldKind, f64)>, bool);

/// Read one click of `sketch.draw`.
fn click_of(click: &Value) -> Result<Click, CommandError> {
    let at = points(Some(&json!([click])))
        .map_err(|_| CommandError::bad("points", "must hold {x, y} clicks"))?[0];
    let mut typed = Vec::new();
    let mut constrain = false;
    if let Value::Object(fields) = click {
        if let Some(Value::Object(values)) = fields.get("typed") {
            for (name, v) in values {
                let kind = crate::step::field_of(name).ok_or_else(|| {
                    CommandError::bad("points", format!("types `{name}`, which no tool asks for"))
                })?;
                let v = v
                    .as_f64()
                    .ok_or_else(|| CommandError::bad("points", "typed values are numbers"))?;
                typed.push((kind, v));
            }
        }
        constrain = fields
            .get("constrain")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    }
    Ok((at, typed, constrain))
}

/// The dimension of `sketch` that already measures what `kind` would make
/// of `items` ([`crate::constrain::dimension_held`]).
pub(crate) fn held_dimension(sketch: &Sketch, kind: &str, items: &[Uuid]) -> Option<Uuid> {
    let (_, kinds) = planned(sketch, kind, items).ok()?;
    kinds
        .iter()
        .find_map(|k| crate::constrain::dimension_held(sketch, k))
}

/// The tool `kind` stands for on `items` and the constraints it makes.
fn planned<'a>(
    sketch: &Sketch,
    kind: &'a str,
    items: &[Uuid],
) -> Result<(&'a str, Vec<crate::sketch::ConstraintKind>), CommandError> {
    if kind != "dimension" && !crate::constrain::TOOLS.contains(&kind) {
        return Err(CommandError::bad(
            "kind",
            format!(
                "has `{kind}`, which is no constraint; the kinds are dimension, {}",
                crate::constrain::TOOLS.join(", ")
            ),
        ));
    }
    let selected: std::collections::HashSet<Uuid> = items.iter().copied().collect();
    let shape = crate::constrain::SelectionShape::picked_in(sketch, &selected, items);
    let tool = if kind == "dimension" {
        crate::constrain::dimension_in(&shape, sketch)
            .ok_or_else(|| CommandError::failed("dimension takes a line, circles, or two items"))?
    } else {
        kind
    };
    let kinds = crate::constrain::kinds_for(tool, &shape, sketch).ok_or_else(|| {
        CommandError::failed(format!("the {tool} constraint does not fit these items"))
    })?;
    Ok((tool, kinds))
}

/// Add the constraints `kind` makes for `items`, at `value` when given.
pub(crate) fn constrain(
    sketch: &mut Sketch,
    kind: &str,
    items: &[Uuid],
    value: Option<f64>,
) -> Result<Vec<String>, CommandError> {
    let (tool, kinds) = planned(sketch, kind, items)?;
    // A value goes to the dimension among them; the constraints that come
    // with it (a point kept on a curve, the equal radii) take none.
    if value.is_some()
        && !kinds
            .iter()
            .any(|k| crate::sketch::dimension_value(k).is_some())
    {
        return Err(CommandError::bad("value", format!("{tool} takes no value")));
    }
    if let Some(held) = kinds
        .iter()
        .find_map(|k| crate::constrain::dimension_held(sketch, k))
    {
        return Err(CommandError::failed(format!(
            "the sketch already holds this dimension ({held}); change its value instead"
        )));
    }
    let mut made = Vec::new();
    for mut k in kinds {
        if let Some(value) = value
            && crate::sketch::dimension_value(&k).is_some()
        {
            k = crate::sketch::with_dimension_value(&k, value);
        }
        made.push(sketch.add_constraint(k).to_string());
    }
    Ok(made)
}

/// A list of `{x, y}` pairs, as `{{0, 0}, {10, 0}}` or `{{x = 0, y = 0}}`.
fn points(value: Option<&Value>) -> Result<Vec<Vec2D>, CommandError> {
    let bad = || CommandError::bad("points", "must be a list of {x, y} pairs");
    let list = value.and_then(Value::as_array).ok_or_else(bad)?;
    list.iter()
        .map(|p| {
            let (x, y) = match p {
                Value::Array(xy) if xy.len() == 2 => (xy[0].as_f64(), xy[1].as_f64()),
                Value::Object(xy) => (
                    xy.get("x").and_then(Value::as_f64),
                    xy.get("y").and_then(Value::as_f64),
                ),
                _ => (None, None),
            };
            Ok(vec(x.ok_or_else(bad)?, y.ok_or_else(bad)?))
        })
        .collect()
}

/// The sketch's pictures as an export reads them, their edges found now
/// when they are not yet.
pub(crate) fn pictures<'a>(
    sketch: &'a Sketch,
    ctx: &WorkbenchRuntimeContext,
    edges: &crate::picture_edges::Edges,
) -> Vec<crate::drawing::Picture<'a>> {
    sketch
        .images
        .iter()
        .map(|image| {
            let bytes = || ctx.document.asset_bytes(image.asset).map(<[u8]>::to_vec);
            crate::drawing::Picture {
                image,
                pixels: ctx
                    .document
                    .asset_bytes(image.asset)
                    .and_then(crate::images::pixel_size),
                edges: edges.now(image.asset, bytes),
            }
        })
        .collect()
}
