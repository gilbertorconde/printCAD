//! The pictures an agent or a script asks for to check its work: the
//! `view` tool and `doc.picture`. A request names a view (the user's
//! direction, a standard one, two angles or a direction), the bodies to
//! show, faces and edges to paint, markers, a section, see-through bodies
//! and annotations; `picture` draws it through the CPU renderer in
//! `thumbnail.rs`, framed to what is drawn. Every view but `current`
//! depends on nothing but the document and the request, so the same
//! request draws the same PNG whatever the user's camera.

use std::sync::{Arc, LazyLock};

use axes::AxisSystem;
use core_document::{BodyId, CommandSpec, Document, ParamKind};
use glam::Vec3;
use serde_json::{Map, Value, json};
use ui_kit::tokens;

use crate::thumbnail::{self, Look, Projection, Shape};

/// The picture's size when the request names none.
pub const DEFAULT_SIZE: (u32, u32) = (800, 600);
/// The largest side a picture may have.
pub const MAX_SIDE: u32 = 2048;
const MIN_SIDE: u32 = 16;
/// The standard isometric elevation: equal angles to the three axes.
const ISO_ELEVATION: f32 = 35.264_39;

/// Where a picture looks from.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewSpec {
    /// The direction the user's view looks.
    Current,
    /// Degrees about the vertical axis (0 the front, 90 the right side)
    /// and above the horizon (90 straight down).
    Angles { azimuth: f32, elevation: f32 },
    /// The way the view looks, and the direction up on the picture.
    Direction { direction: Vec3, up: Option<Vec3> },
}

/// A body's faces and edges to paint, or the whole body when it names
/// neither.
#[derive(Clone, Debug, Default)]
pub struct Highlight {
    pub body: String,
    pub faces: Vec<FaceKey>,
    pub edges: Vec<u32>,
    pub color: Option<[f32; 3]>,
}

/// A face as `doc.faces` gives it: its index, or its name.
#[derive(Clone, Debug)]
pub enum FaceKey {
    Index(u32),
    Name(String),
}

/// A dot, and a label beside it, at a world point.
#[derive(Clone, Debug)]
pub struct Marker {
    pub point: Vec3,
    pub label: Option<String>,
    pub color: Option<[f32; 3]>,
}

/// What to draw and how.
#[derive(Clone, Debug)]
pub struct Request {
    pub view: ViewSpec,
    /// Only these bodies, by id or name; every visible one when `None`.
    pub bodies: Option<Vec<String>>,
    pub highlight: Vec<Highlight>,
    pub markers: Vec<Marker>,
    /// A point on the cutting plane and its normal: what lies on the side
    /// the normal points to is cut away.
    pub section: Option<(Vec3, Vec3)>,
    pub edges: bool,
    pub xray: bool,
    pub size: (u32, u32),
    pub annotate: bool,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            view: ViewSpec::Current,
            bodies: None,
            highlight: Vec::new(),
            markers: Vec::new(),
            section: None,
            edges: true,
            xray: false,
            size: DEFAULT_SIZE,
            annotate: false,
        }
    }
}

/// The request's arguments, documented once for the `view` tool's schema
/// and `doc.picture`'s spec: name, kind, schema and words.
fn arguments() -> Vec<(&'static str, ParamKind, Value, &'static str)> {
    let point = json!({"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3});
    let colour = json!({"description": "[r, g, b] from 0 to 1, or \"#rrggbb\""});
    vec![
        (
            "view",
            ParamKind::Any,
            json!({"oneOf": [
                {"type": "string", "enum": ["current", "iso", "front", "back", "left", "right", "top", "bottom"]},
                {"type": "object", "properties": {
                    "azimuth": {"type": "number"}, "elevation": {"type": "number"}}},
                {"type": "object", "properties": {"direction": point, "up": point},
                 "required": ["direction"]}
            ]}),
            "Where to look from: \"current\" (the user's view direction; the default), \
             \"iso\", \"front\", \"back\", \"left\", \"right\", \"top\", \"bottom\", \
             {azimuth, elevation} in degrees (azimuth 0 the front, 90 the right side; \
             elevation 90 from straight above), or {direction, up?}, the way the view \
             looks. Always orthographic and framed to what is drawn",
        ),
        (
            "bodies",
            ParamKind::List,
            json!({"type": "array", "items": {"type": "string"}}),
            "Only these bodies (ids or names), framed to them; every visible body when \
             left out",
        ),
        (
            "highlight",
            ParamKind::List,
            json!({"type": "array", "items": {"type": "object", "properties": {
                "body": {"type": "string"},
                "faces": {"type": "array", "items": {"type": ["integer", "string"]}},
                "edges": {"type": "array", "items": {"type": "integer"}},
                "color": colour
            }, "required": ["body"]}}),
            "Paint: a list of {body, faces?, edges?, color?}; faces by doc.faces index or \
             name, edges by doc.edges index; the whole body when neither is given",
        ),
        (
            "markers",
            ParamKind::List,
            json!({"type": "array", "items": {"type": "object", "properties": {
                "point": point, "label": {"type": "string"}, "color": colour
            }, "required": ["point"]}}),
            "A dot and a label at world points: a list of {point, label?, color?}",
        ),
        (
            "section",
            ParamKind::Any,
            json!({"type": "object", "properties": {"origin": point, "normal": point},
                   "required": ["origin", "normal"]}),
            "Cut at the plane {origin, normal}: what lies on the side the normal points to \
             is cut away, the cut drawn flat and darker",
        ),
        (
            "edges",
            ParamKind::Bool,
            json!({"type": "boolean"}),
            "Draw the faces' outlines (true)",
        ),
        (
            "xray",
            ParamKind::Bool,
            json!({"type": "boolean"}),
            "Bodies see-through, painted faces solid (false)",
        ),
        (
            "size",
            ParamKind::List,
            json!({"type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2}),
            "[width, height] in pixels (800 × 600, at most 2048 a side)",
        ),
        (
            "annotate",
            ParamKind::Bool,
            json!({"type": "boolean"}),
            "Draw an axis triad and the drawn bodies' box with its sizes (false)",
        ),
    ]
}

/// The `view` tool's input schema.
#[cfg(not(target_arch = "wasm32"))]
pub fn schema() -> Value {
    let properties: Map<String, Value> = arguments()
        .into_iter()
        .map(|(name, _, mut schema, words)| {
            schema["description"] = json!(words);
            (name.to_string(), schema)
        })
        .collect();
    json!({"type": "object", "properties": properties})
}

/// `spec` with the request's arguments added, each optional.
pub fn with_arguments(mut spec: CommandSpec) -> CommandSpec {
    for (name, kind, _, words) in arguments() {
        spec = spec.optional(name, kind, words);
    }
    spec
}

/// The request in `args`; arguments it does not know are left for the
/// caller (`doc.picture`'s `path`).
pub fn parse(args: &Map<String, Value>) -> Result<Request, String> {
    let mut request = Request::default();
    let given = |name: &str| args.get(name).filter(|v| !v.is_null());
    if let Some(view) = given("view") {
        request.view = parse_view(view)?;
    }
    if let Some(bodies) = given("bodies") {
        request.bodies = Some(
            list(bodies, "bodies")?
                .iter()
                .map(|v| text(v, "bodies"))
                .collect::<Result<_, _>>()?,
        );
    }
    if let Some(highlight) = given("highlight") {
        for item in list(highlight, "highlight")? {
            let item = item
                .as_object()
                .ok_or("highlight: each entry is {body, faces?, edges?, color?}")?;
            let body = text(
                item.get("body")
                    .ok_or("highlight: each entry names a body")?,
                "highlight.body",
            )?;
            let mut faces = Vec::new();
            if let Some(list_of) = item.get("faces") {
                for face in list(list_of, "highlight.faces")? {
                    faces.push(match face {
                        Value::String(name) => FaceKey::Name(name.clone()),
                        other => FaceKey::Index(index(other, "highlight.faces")?),
                    });
                }
            }
            let mut edges = Vec::new();
            if let Some(list_of) = item.get("edges") {
                for edge in list(list_of, "highlight.edges")? {
                    edges.push(index(edge, "highlight.edges")?);
                }
            }
            let color = item.get("color").map(colour).transpose()?;
            request.highlight.push(Highlight {
                body,
                faces,
                edges,
                color,
            });
        }
    }
    if let Some(markers) = given("markers") {
        for item in list(markers, "markers")? {
            let item = item
                .as_object()
                .ok_or("markers: each entry is {point, label?, color?}")?;
            request.markers.push(Marker {
                point: vector(
                    item.get("point").ok_or("markers: each entry has a point")?,
                    "markers.point",
                )?,
                label: item.get("label").filter(|v| !v.is_null()).map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                }),
                color: item.get("color").map(colour).transpose()?,
            });
        }
    }
    if let Some(section) = given("section") {
        let origin = section
            .get("origin")
            .ok_or("section: {origin, normal}, both [x, y, z]")?;
        let normal = section
            .get("normal")
            .ok_or("section: {origin, normal}, both [x, y, z]")?;
        let normal = vector(normal, "section.normal")?;
        if normal.length_squared() < 1e-12 {
            return Err("section.normal: must not be zero".into());
        }
        request.section = Some((vector(origin, "section.origin")?, normal));
    }
    for (name, slot) in [
        ("edges", &mut request.edges),
        ("xray", &mut request.xray),
        ("annotate", &mut request.annotate),
    ] {
        if let Some(v) = given(name) {
            *slot = v
                .as_bool()
                .ok_or_else(|| format!("{name}: must be true or false"))?;
        }
    }
    if let Some(size) = given("size") {
        let sides = list(size, "size")?;
        let [w, h] = sides else {
            return Err("size: [width, height]".into());
        };
        let side = |v: &Value| -> Result<u32, String> {
            let n = v.as_f64().ok_or("size: [width, height] in pixels")?;
            Ok((n.round().max(0.0) as u32).clamp(MIN_SIDE, MAX_SIDE))
        };
        request.size = (side(w)?, side(h)?);
    }
    Ok(request)
}

fn parse_view(value: &Value) -> Result<ViewSpec, String> {
    let named = |azimuth: f32, elevation: f32| ViewSpec::Angles { azimuth, elevation };
    match value {
        Value::String(name) => Ok(match name.to_ascii_lowercase().as_str() {
            "current" => ViewSpec::Current,
            "iso" | "isometric" => named(45.0, ISO_ELEVATION),
            "front" => named(0.0, 0.0),
            "back" | "rear" => named(180.0, 0.0),
            "right" => named(90.0, 0.0),
            "left" => named(-90.0, 0.0),
            "top" => named(0.0, 90.0),
            "bottom" => named(0.0, -90.0),
            other => {
                return Err(format!(
                    "view: `{other}` is not a view; current, iso, front, back, left, right, \
                     top, bottom, {{azimuth, elevation}} or {{direction, up?}}"
                ));
            }
        }),
        Value::Object(map) if map.contains_key("direction") => {
            let direction = vector(&map["direction"], "view.direction")?;
            if direction.length_squared() < 1e-12 {
                return Err("view.direction: must not be zero".into());
            }
            let up = map
                .get("up")
                .filter(|v| !v.is_null())
                .map(|v| vector(v, "view.up"))
                .transpose()?;
            Ok(ViewSpec::Direction { direction, up })
        }
        Value::Object(map) => {
            let angle = |name: &str| -> Result<f32, String> {
                match map.get(name) {
                    None | Some(Value::Null) => Ok(0.0),
                    Some(v) => v
                        .as_f64()
                        .map(|n| n as f32)
                        .ok_or_else(|| format!("view.{name}: degrees")),
                }
            };
            Ok(ViewSpec::Angles {
                azimuth: angle("azimuth")?,
                elevation: angle("elevation")?.clamp(-90.0, 90.0),
            })
        }
        _ => Err("view: a name, {azimuth, elevation} or {direction, up?}".into()),
    }
}

/// A list, as JSON gives it or as a script's table does (an empty table
/// may come as an object).
fn list<'a>(value: &'a Value, name: &str) -> Result<&'a [Value], String> {
    match value {
        Value::Array(items) => Ok(items),
        Value::Object(map) if map.is_empty() => Ok(&[]),
        _ => Err(format!("{name}: must be a list")),
    }
}

fn text(value: &Value, name: &str) -> Result<String, String> {
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{name}: must be a string"))
}

fn index(value: &Value, name: &str) -> Result<u32, String> {
    value
        .as_f64()
        .filter(|n| *n >= 0.0 && n.fract() == 0.0 && *n <= f64::from(u32::MAX))
        .map(|n| n as u32)
        .ok_or_else(|| format!("{name}: indices are whole numbers from 0"))
}

fn vector(value: &Value, name: &str) -> Result<Vec3, String> {
    let numbers: Option<Vec<f32>> = value
        .as_array()
        .and_then(|items| items.iter().map(|v| v.as_f64().map(|n| n as f32)).collect());
    match numbers.as_deref() {
        Some([x, y, z]) => Ok(Vec3::new(*x, *y, *z)),
        _ => Err(format!("{name}: must be [x, y, z]")),
    }
}

fn colour(value: &Value) -> Result<[f32; 3], String> {
    const BAD: &str = "color: [r, g, b] from 0 to 1, or \"#rrggbb\"";
    match value {
        Value::String(hex) => {
            let digits = hex.strip_prefix('#').unwrap_or(hex);
            let n = u32::from_str_radix(digits, 16).map_err(|_| BAD)?;
            if digits.len() != 6 {
                return Err(BAD.into());
            }
            Ok([16, 8, 0].map(|s| ((n >> s) & 0xFF) as f32 / 255.0))
        }
        Value::Array(items) => match items.as_slice() {
            [r, g, b] => {
                let c = [r, g, b].map(|v| v.as_f64().map(|n| (n as f32).clamp(0.0, 1.0)));
                match c {
                    [Some(r), Some(g), Some(b)] => Ok([r, g, b]),
                    _ => Err(BAD.into()),
                }
            }
            _ => Err(BAD.into()),
        },
        _ => Err(BAD.into()),
    }
}

/// The view's forward and up directions in world space. Angles are taken
/// in the axis preset's frame, so "front" and "top" mean what the view's
/// own standard views mean.
pub fn basis(view: &ViewSpec, current: Option<(Vec3, Vec3)>, axes: AxisSystem) -> (Vec3, Vec3) {
    let (right, vertical, depth) = (axes.right_vec(), axes.up_vec(), axes.forward_vec());
    let angles = |azimuth: f32, elevation: f32| {
        let (az, el) = (azimuth.to_radians(), elevation.to_radians());
        let level = depth * az.cos() + right * az.sin();
        let eye = level * el.cos() + vertical * el.sin();
        let up = vertical * el.cos() - level * el.sin();
        (-eye, up)
    };
    match view {
        ViewSpec::Current => current.unwrap_or_else(|| angles(45.0, ISO_ELEVATION)),
        ViewSpec::Angles { azimuth, elevation } => angles(*azimuth, *elevation),
        ViewSpec::Direction { direction, up } => {
            let forward = direction.normalize();
            let up = up
                .filter(|u| forward.cross(*u).length_squared() > 1e-8)
                .unwrap_or(if forward.cross(vertical).length_squared() > 1e-8 {
                    vertical
                } else if forward.dot(vertical) < 0.0 {
                    -depth
                } else {
                    depth
                });
            (forward, up)
        }
    }
}

/// The colours a picture draws in beside the bodies' own.
fn rgb(colour: egui::Color32) -> [f32; 3] {
    [colour.r(), colour.g(), colour.b()].map(|c| f32::from(c) / 255.0)
}

fn hex(colour: [f32; 3]) -> String {
    let [r, g, b] = colour.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The PNG `request` asks for of `document`. `current` is the user's view
/// direction (forward, up), where there is one; without it `current`
/// draws the isometric view.
pub fn picture(
    document: &Document,
    request: &Request,
    current: Option<(Vec3, Vec3)>,
    axes: AxisSystem,
) -> Result<Vec<u8>, String> {
    // The bodies, in a fixed order, so equal depths always fall alike.
    let mut drawn: Vec<(BodyId, Shape)> = match &request.bodies {
        Some(names) => {
            let mut ids = Vec::new();
            for name in names {
                let id = find_body(document, name)?;
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            ids.into_iter()
                .map(|id| {
                    let geometry = document.imported_geometry(id).ok_or_else(|| {
                        format!(
                            "body `{}` has no shape yet; run doc.rebuild first",
                            name_of(document, id)
                        )
                    })?;
                    Ok((id, shape_of(document, id, &geometry.mesh)))
                })
                .collect::<Result<_, String>>()?
        }
        None => document
            .imported_geometries()
            .filter(|(id, _)| document.imported_body_effective_visible(**id))
            .map(|(id, geometry)| (*id, shape_of(document, *id, &geometry.mesh)))
            .collect(),
    };
    drawn.sort_by_key(|(id, _)| *id);

    let paint_default = rgb(tokens::WARNING);
    let mut look = Look {
        edges: request
            .edges
            .then(|| settings::LightingSettings::default().edge_line_color),
        section: request.section,
        xray: request.xray,
        background: Some((rgb(tokens::VIEWPORT_TOP), rgb(tokens::VIEWPORT_BOTTOM))),
        frame_points: request.markers.iter().map(|m| m.point).collect(),
        ..Look::default()
    };
    for highlight in &request.highlight {
        let id = find_body(document, &highlight.body)?;
        let Some(si) = drawn.iter().position(|(b, _)| *b == id) else {
            return Err(format!(
                "highlight: body `{}` is not drawn; add it to bodies or show it",
                highlight.body
            ));
        };
        let colour = highlight.color.unwrap_or(paint_default);
        let mesh = Arc::clone(&drawn[si].1.mesh);
        let face_count = mesh.face_surfaces.len().max(
            mesh.faces
                .iter()
                .map(|f| *f as usize + 1)
                .max()
                .unwrap_or(0),
        );
        if highlight.faces.is_empty() && highlight.edges.is_empty() {
            if face_count > 0 {
                for f in 0..face_count as u32 {
                    look.face_paint.insert((si, f), colour);
                }
            } else {
                drawn[si].1.color = colour;
                drawn[si].1.vertex_colours = false;
            }
        }
        for face in &highlight.faces {
            let f = match face {
                FaceKey::Index(i) if (*i as usize) < face_count => *i,
                FaceKey::Index(i) => {
                    return Err(format!(
                        "highlight: body `{}` has no face {i}; it has {face_count} (doc.faces)",
                        highlight.body
                    ));
                }
                FaceKey::Name(name) => name
                    .parse::<u64>()
                    .ok()
                    .and_then(|n| mesh.face_names.iter().position(|m| *m == n && n != 0))
                    .map(|i| i as u32)
                    .ok_or_else(|| {
                        format!(
                            "highlight: body `{}` has no face named `{name}` (doc.faces)",
                            highlight.body
                        )
                    })?,
            };
            look.face_paint.insert((si, f), colour);
        }
        for edge in &highlight.edges {
            if !mesh.edge_ids.contains(edge) {
                return Err(format!(
                    "highlight: body `{}` has no edge {edge}",
                    highlight.body
                ));
            }
            look.edge_paint.insert((si, *edge), colour);
        }
    }

    let shapes: Vec<Shape> = drawn.into_iter().map(|(_, s)| s).collect();
    let (forward, up) = basis(&request.view, current, axes);
    let (width, height) = request.size;
    let picture = thumbnail::draw(&shapes, &shapes, forward, up, width, height, &look)
        .ok_or("Nothing is visible to draw.")?;
    let mut pixmap =
        thumbnail::pixmap(&picture.rgba, width, height).ok_or("the picture could not be made")?;
    let overlay = overlay(request, &shapes, &picture.projection);
    if !overlay.is_empty() {
        draw_svg(&mut pixmap, width, height, &overlay)?;
    }
    pixmap.encode_png().map_err(|e| e.to_string())
}

fn shape_of(document: &Document, id: BodyId, mesh: &Arc<kernel_api::TriMesh>) -> Shape {
    crate::app::doc_io::preview_shape(document, id, Arc::clone(mesh))
}

fn find_body(document: &Document, key: &str) -> Result<BodyId, String> {
    let bodies = document.bodies();
    if let Ok(uuid) = uuid::Uuid::parse_str(key) {
        let id = BodyId(uuid);
        if bodies.iter().any(|b| b.id == id) || document.imported_geometry(id).is_some() {
            return Ok(id);
        }
    }
    bodies
        .iter()
        .find(|b| b.name == key)
        .map(|b| b.id)
        .ok_or_else(|| format!("`{key}` is not a body of this document (doc.bodies)"))
}

fn name_of(document: &Document, id: BodyId) -> String {
    document
        .bodies()
        .iter()
        .find(|b| b.id == id)
        .map_or_else(|| id.0.to_string(), |b| b.name.clone())
}

/// The markers and annotations as SVG drawn over the picture; empty when
/// there are none.
fn overlay(request: &Request, shapes: &[Shape], projection: &Projection) -> String {
    let mut svg = String::new();
    let label = |svg: &mut String, x: f32, y: f32, words: &str, fill: [f32; 3]| {
        svg.push_str(&format!(
            "<text x=\"{x:.1}\" y=\"{y:.1}\" font-size=\"{size}\" fill=\"{fill}\" \
             stroke=\"{halo}\" stroke-width=\"3\" stroke-linejoin=\"round\" \
             paint-order=\"stroke\">{words}</text>",
            size = tokens::FONT_MD,
            fill = hex(fill),
            halo = hex(rgb(tokens::BG0)),
            words = escape(words),
        ));
    };
    if request.annotate {
        // The box of what is drawn, dashed, its sizes above the picture.
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for shape in shapes {
            for p in &shape.mesh.positions {
                lo = lo.min(Vec3::from(*p));
                hi = hi.max(Vec3::from(*p));
            }
        }
        if lo.x <= hi.x {
            let corner = |i: u32| {
                Vec3::new(
                    if i & 1 == 0 { lo.x } else { hi.x },
                    if i & 2 == 0 { lo.y } else { hi.y },
                    if i & 4 == 0 { lo.z } else { hi.z },
                )
            };
            let colour = hex(rgb(tokens::ANNOTATION));
            for i in 0..8u32 {
                for bit in [1, 2, 4] {
                    if i & bit == 0 {
                        let (a, b) = (
                            projection.to_pixel(corner(i)),
                            projection.to_pixel(corner(i | bit)),
                        );
                        svg.push_str(&format!(
                            "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" \
                             stroke=\"{colour}\" stroke-width=\"1\" stroke-dasharray=\"4 3\"/>",
                            a.0, a.1, b.0, b.1
                        ));
                    }
                }
            }
            let size = hi - lo;
            let words = format!(
                "X {} × Y {} × Z {} mm",
                number(size.x),
                number(size.y),
                number(size.z)
            );
            label(&mut svg, 12.0, 22.0, &words, rgb(tokens::ANNOTATION));
        }
        // The axis triad, bottom left, the axes leaning away drawn first.
        let origin = (44.0, projection.height as f32 - 44.0);
        let mut axes = [
            (Vec3::X, "X", rgb(tokens::AXIS_X)),
            (Vec3::Y, "Y", rgb(tokens::AXIS_Y)),
            (Vec3::Z, "Z", rgb(tokens::AXIS_Z)),
        ];
        axes.sort_by(|a, b| {
            b.0.dot(projection.forward)
                .total_cmp(&a.0.dot(projection.forward))
        });
        for (axis, name, colour) in axes {
            let d = (axis.dot(projection.right), -axis.dot(projection.up));
            let end = (origin.0 + d.0 * 28.0, origin.1 + d.1 * 28.0);
            svg.push_str(&format!(
                "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" stroke=\"{}\" \
                 stroke-width=\"2.5\" stroke-linecap=\"round\"/>",
                origin.0,
                origin.1,
                end.0,
                end.1,
                hex(colour)
            ));
            label(
                &mut svg,
                origin.0 + d.0 * 36.0 - 4.0,
                origin.1 + d.1 * 36.0 + 5.0,
                name,
                colour,
            );
        }
    }
    for marker in &request.markers {
        let (x, y) = projection.to_pixel(marker.point);
        let colour = marker.color.unwrap_or(rgb(tokens::DANGER));
        svg.push_str(&format!(
            "<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"5\" fill=\"{}\" stroke=\"{}\" \
             stroke-width=\"2\"/>",
            hex(colour),
            hex(rgb(tokens::BG0)),
        ));
        if let Some(words) = &marker.label {
            label(&mut svg, x + 9.0, y - 7.0, words, rgb(tokens::TEXT1));
        }
    }
    svg
}

/// A length as a label shows it: two decimals at most, trailing zeros off.
fn number(v: f32) -> String {
    let text = format!("{v:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn escape(words: &str) -> String {
    words
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The bundled sans face only: no system font scan, and the same glyphs on
/// every machine.
static FONTS: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
    let mut fonts = usvg::fontdb::Database::new();
    fonts.load_font_data(ui_kit::theme::SANS_REGULAR_TTF.to_vec());
    Arc::new(fonts)
});

fn draw_svg(
    pixmap: &mut tiny_skia::Pixmap,
    width: u32,
    height: u32,
    body: &str,
) -> Result<(), String> {
    let family = FONTS
        .faces()
        .next()
        .and_then(|f| f.families.first())
        .map(|(name, _)| name.clone())
        .unwrap_or_default();
    let source = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         font-family=\"{family}\">{body}</svg>"
    );
    let options = usvg::Options {
        fontdb: Arc::clone(&FONTS),
        font_family: family,
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(source.as_bytes(), &options).map_err(|e| e.to_string())?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn requests_read_their_arguments_and_refuse_wrong_ones() {
        let r = parse(&args(json!({
            "view": {"azimuth": 30, "elevation": 20},
            "highlight": [{"body": "Body", "faces": [0, "123"], "edges": [2], "color": "#ff0000"}],
            "markers": [{"point": [1, 2, 3], "label": "centre"}],
            "section": {"origin": [0, 0, 5], "normal": [0, 0, 1]},
            "size": [5000, 10],
            "edges": false,
        })))
        .unwrap();
        assert_eq!(
            r.view,
            ViewSpec::Angles {
                azimuth: 30.0,
                elevation: 20.0
            }
        );
        assert_eq!(r.size, (MAX_SIDE, MIN_SIDE));
        assert!(!r.edges);
        assert_eq!(r.highlight[0].color, Some([1.0, 0.0, 0.0]));
        assert!(matches!(r.highlight[0].faces[1], FaceKey::Name(_)));
        assert!(parse(&args(json!({"view": "sideways"}))).is_err());
        assert!(parse(&args(json!({"section": {"origin": [0, 0, 0]}}))).is_err());
        assert!(parse(&args(json!({"markers": [{"label": "x"}]}))).is_err());
    }

    #[test]
    fn standard_views_follow_the_axis_preset() {
        let z_up = AxisSystem::default();
        let close = |a: Vec3, b: Vec3| a.distance(b) < 1e-5;
        let view = |name: &str| parse_view(&json!(name)).unwrap();
        // Z up: the front looks along +Y, the right side along -X, the top
        // down -Z with +Y up the picture.
        let (f, u) = basis(&view("front"), None, z_up);
        assert!(close(f, Vec3::Y) && close(u, Vec3::Z));
        let (f, _) = basis(&view("right"), None, z_up);
        assert!(close(f, -Vec3::X));
        let (f, u) = basis(&view("top"), None, z_up);
        assert!(close(f, -Vec3::Z) && close(u, Vec3::Y));
        // The right side is on the right in the top view.
        assert!(close(f.cross(u), Vec3::X));
        // Y up: the top looks down -Y.
        let y_up = AxisSystem::from_preset(axes::AxisPreset::RightHandedZForward);
        let (f, _) = basis(&view("top"), None, y_up);
        assert!(close(f, -Vec3::Y));
        // A direction straight down takes the top view's up.
        let (_, u) = basis(
            &ViewSpec::Direction {
                direction: -Vec3::Z,
                up: None,
            },
            None,
            z_up,
        );
        assert!(close(u, Vec3::Y));
    }

    /// A document with one 10 mm box at the origin, its faces numbered
    /// bottom, top, front (y = 0), right (x = 10), back, left.
    fn boxed() -> (Document, BodyId) {
        let mut document = Document::new("box");
        let body = document.create_body(Some("Box".into()));
        let cube = crate::thumbnail::tests::cube(0.0);
        let mut mesh = (*cube.mesh).clone();
        for p in &mut mesh.positions {
            *p = p.map(|c| c * 10.0);
        }
        document.set_imported_geometry(
            body,
            core_document::ImportedGeometry {
                mesh: Arc::new(mesh),
                source_asset: None,
                revision: 0,
                bounds_mm: None,
                brep_blob_path: None,
                mesh_path: None,
                face_colors_path: None,
                health: None,
            },
        );
        (document, body)
    }

    fn draw(document: &Document, request: Value) -> tiny_skia::Pixmap {
        let request = parse(&args(request)).unwrap();
        let png = picture(document, &request, None, AxisSystem::default()).unwrap();
        tiny_skia::Pixmap::decode_png(&png).unwrap()
    }

    /// Straight RGB at a pixel of an opaque picture.
    fn at(pixmap: &tiny_skia::Pixmap, x: u32, y: u32) -> [u8; 3] {
        let p = pixmap.pixel(x, y).unwrap();
        [p.red(), p.green(), p.blue()]
    }

    fn is_background(pixmap: &tiny_skia::Pixmap, x: u32, y: u32) -> bool {
        let (top, bottom) = (rgb(tokens::VIEWPORT_TOP), rgb(tokens::VIEWPORT_BOTTOM));
        let t = (y as f32 + 0.5) / pixmap.height() as f32;
        let expected = [0, 1, 2].map(|c| ((top[c] + (bottom[c] - top[c]) * t) * 255.0).round());
        at(pixmap, x, y)
            .iter()
            .zip(expected)
            .all(|(got, want)| (f32::from(*got) - want).abs() <= 1.0)
    }

    #[test]
    fn the_same_request_draws_the_same_png_whatever_the_camera() {
        let (document, body) = boxed();
        let request = parse(&args(json!({
            "view": "iso",
            "highlight": [{"body": body.0.to_string(), "faces": [1]}],
            "markers": [{"point": [10, 10, 10], "label": "corner"}],
            "annotate": true,
        })))
        .unwrap();
        let axes = AxisSystem::default();
        let one = picture(&document, &request, Some((Vec3::X, Vec3::Z)), axes).unwrap();
        let other = picture(&document, &request, Some((-Vec3::Z, Vec3::Y)), axes).unwrap();
        assert!(one.starts_with(b"\x89PNG"));
        assert!(one == other, "the same bytes");
        // The user's direction does change a `current` picture: the front
        // face, painted, shows from the front and not from the side.
        let current = parse(&args(json!({
            "highlight": [{"body": "Box", "faces": [2]}]
        })))
        .unwrap();
        let front = picture(&document, &current, Some((Vec3::Y, Vec3::Z)), axes).unwrap();
        let side = picture(&document, &current, Some((-Vec3::X, Vec3::Z)), axes).unwrap();
        assert!(
            front != side,
            "the user's direction moves a current picture"
        );
    }

    #[test]
    fn a_highlighted_face_takes_the_highlight_colour() {
        let (document, body) = boxed();
        let plain = draw(&document, json!({"view": "front", "edges": false}));
        let painted = draw(
            &document,
            json!({"view": "front", "edges": false,
                   "highlight": [{"body": "Box", "faces": [2], "color": [1, 0, 0]}]}),
        );
        let [r, g, b] = at(&painted, 400, 300);
        assert!(
            r > 150 && g < 40 && b < 40,
            "the front face is red: {:?}",
            [r, g, b]
        );
        let [r, g, b] = at(&plain, 400, 300);
        assert!(
            r.abs_diff(b) < 40 && r.abs_diff(g) < 40,
            "unpainted it is grey"
        );
        // A face the body does not have is refused.
        let request = parse(&args(json!({
            "highlight": [{"body": body.0.to_string(), "faces": [9]}]
        })))
        .unwrap();
        assert!(picture(&document, &request, None, AxisSystem::default()).is_err());
    }

    #[test]
    fn a_section_cuts_away_what_lies_past_the_plane() {
        let (document, _) = boxed();
        let section = json!({"origin": [0, 0, 5], "normal": [0, 0, 1]});
        let cut = draw(&document, json!({"view": "front", "section": section}));
        let whole = draw(&document, json!({"view": "front"}));
        // Framed to the whole box: 600 × 0.84 px for 10 mm, centred. The
        // upper half is gone, the lower half stays.
        let upper = 300 - 126;
        let lower = 300 + 126;
        assert!(!is_background(&whole, 400, upper));
        assert!(is_background(&cut, 400, upper));
        assert!(!is_background(&cut, 400, lower));
        // From above, the cut shows as the darker section fill.
        let top = draw(
            &document,
            json!({"view": "top", "section": section, "edges": false}),
        );
        let [r, g, b] = at(&top, 400, 300);
        let grey = core_document::BodyDisplay::default().color;
        let expected = grey.map(|c| (c * 0.55 * 255.0).round() as u8);
        assert!(
            [r, g, b]
                .iter()
                .zip(expected)
                .all(|(a, e)| a.abs_diff(e) <= 2),
            "section fill {:?}, expected {expected:?}",
            [r, g, b]
        );
    }

    #[test]
    fn framing_keeps_the_body_inside_a_margin() {
        let (document, _) = boxed();
        for view in ["iso", "front", "top", "right"] {
            let pixmap = draw(&document, json!({"view": view, "size": [400, 300]}));
            let (w, h) = (pixmap.width(), pixmap.height());
            let (mut min_x, mut min_y, mut max_x, mut max_y) = (w, h, 0, 0);
            for y in 0..h {
                for x in 0..w {
                    if !is_background(&pixmap, x, y) {
                        min_x = min_x.min(x);
                        max_x = max_x.max(x);
                        min_y = min_y.min(y);
                        max_y = max_y.max(y);
                    }
                }
            }
            let inset = |n: u32| (n as f32 * crate::thumbnail::MARGIN * 0.8) as u32;
            assert!(
                min_x >= inset(w) && max_x < w - inset(w),
                "{view}: {min_x}..{max_x}"
            );
            assert!(
                min_y >= inset(h) && max_y < h - inset(h),
                "{view}: {min_y}..{max_y}"
            );
            // And fills it along one side or the other.
            let fill = |lo: u32, hi: u32, n: u32| (hi - lo) as f32 / n as f32;
            assert!(
                fill(min_x, max_x, w) > 0.8 || fill(min_y, max_y, h) > 0.8,
                "{view} fills the frame"
            );
        }
    }
}
