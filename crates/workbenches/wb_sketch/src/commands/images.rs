//! Reference pictures: laying them on the plane, placing, aligning and
//! calibrating them, and reading their edges, outlines and objects.

use core_document::command::vec2_of;
use core_document::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, FeatureId,
    ParamKind, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::sketch::{Circle, GeometryElement, Sketch, Vec2D};

use super::{add_sketch, editing, line, load, placing, point_at, reading, save, sketch, vec};
use crate::SketchWorkbench;
use crate::feature::SketchFeature;
use crate::sketch::ReferenceImage;

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<SketchWorkbench>) {
    t.add(
        placed(placing(CommandSpec::new(
            "sketch.image",
            "Lay a picture (PNG or JPEG) on a sketch's plane to draw over: in the sketch \
             given or being edited, else in a new one",
        )))
        .param("path", ParamKind::String, "The picture file")
        .optional("sketch", ParamKind::Id, "The sketch it goes in")
        .optional(
            "profile",
            ParamKind::String,
            "A scanner profile to size it by; the one kept for its file's resolution when left out",
        )
        .returns("{sketch, image}")
        .agent_always_asks()
        .note(
            "Without `sketch` it goes in the sketch being edited, else in a new sketch named \
             after the file and placed as `sketch.new` places one (`body`, `plane`, `on`).",
        )
        .note(
            "It lies 100 mm wide, centred on (0, 0), at opacity 0.5, unless `width`, `x`, \
             `y` or `opacity` say otherwise; `angle` is degrees counter-clockwise.",
        )
        .note(
            "The file is kept in the document. The picture only draws: no profile and no \
             constraint come of it.",
        )
        .note(
            "A file that cannot be read, or is not a PNG or JPEG, is refused. The `image` \
             returned is what `sketch.set_image` takes.",
        )
        .see_also("sketch.set_image"),
        |args, ctx| add_image(&Args(args), ctx),
    );
    t.add_free(
        placed(sketch(CommandSpec::new(
            "sketch.set_image",
            "Move, size, turn or fade a sketch's picture, or take it away",
        )))
        .param("image", ParamKind::Id, "The picture")
        .optional("remove", ParamKind::Bool, "true: take it away")
        .note(
            "`image` is the id `sketch.image` returned, and `sketch` must be the sketch \
             holding it, or it is refused.",
        )
        .note(
            "Only what is given changes. `width` must be more than 0, `opacity` is held \
             between 0 and 1, and `angle` is degrees counter-clockwise.",
        )
        .see_also("sketch.image"),
        editing(set_image),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.align_image",
            "Square a picture to the sketch: a point on it to the origin, another along +x",
        ))
        .param(
            "origin",
            ParamKind::Any,
            "{x, y}: the point to be the origin",
        )
        .param(
            "toward",
            ParamKind::Any,
            "{x, y}: a point to lie along +x from it (a sheet's edge, a part's datum)",
        )
        .optional(
            "image",
            ParamKind::Number,
            "Which picture, from 1; the first",
        )
        .returns("{image, turned}")
        .see_also("sketch.turn_image"),
        editing(align_image),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.turn_image",
            "Turn a picture by quarter turns about the origin, or mirror it",
        ))
        .optional(
            "quarters",
            ParamKind::Number,
            "Quarter turns counter-clockwise; 1",
        )
        .optional(
            "mirror",
            ParamKind::Bool,
            "Mirror it across the sketch's y axis: a part scanned face down shown as it is",
        )
        .optional(
            "image",
            ParamKind::Number,
            "Which picture, from 1; the first",
        )
        .returns("{image, angle, mirrored}")
        .see_also("sketch.align_image"),
        editing(turn_image),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.bolt_circle",
            "The circle through the centres of holes, and how far each strays from it",
        ))
        .param(
            "circles",
            ParamKind::List,
            "Three or more of the sketch's circles or arcs",
        )
        .returns("{element, centre = {x, y}, radius, strays = {...}, angles = {...}}")
        .note(
            "Adds the least-squares circle through the circles' centres as construction, with \
             its centre point. `strays` is each hole's distance from it (positive outside), \
             `angles` where each sits round it, degrees from the sketch's x axis.",
        )
        .see_also("sketch.fit_edge"),
        editing(bolt_circle),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.calibrate_axis",
            "Scale a picture along one of its own axes so two points are a known distance apart",
        ))
        .param(
            "from",
            ParamKind::Any,
            "{x, y}: the first point, in the sketch's coordinates; it stays put",
        )
        .param("to", ParamKind::Any, "{x, y}: the second")
        .param(
            "length",
            ParamKind::Number,
            "How far apart they are along the axis, mm",
        )
        .param(
            "axis",
            ParamKind::String,
            "x (the picture's rows) or y (its columns)",
        )
        .optional(
            "image",
            ParamKind::Number,
            "Which picture, from 1; the first",
        )
        .returns("{image, scale}")
        .note(
            "A flatbed scanner's sensor and its carriage may not agree: calibrate along x with \
             a ruler laid across the glass, then along y with one laid down it. The picture's \
             other axis keeps its size.",
        )
        .see_also("sketch.calibrate_image")
        .see_also("sketch.save_scanner_profile"),
        editing(calibrate_axis),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.save_scanner_profile",
            "Keep a calibrated picture's scale as a scanner profile for later pictures",
        ))
        .param(
            "name",
            ParamKind::String,
            "The profile's name: the scanner and its resolution",
        )
        .optional(
            "image",
            ParamKind::Number,
            "Which picture, from 1; the first",
        )
        .returns("{name, dpi, mm_per_px = {x, y}}")
        .agent_always_asks()
        .note(
            "Kept in the configuration folder's scanner_profiles.json. A later picture whose \
             file states the same resolution takes the profile's scale when it is placed \
             without a width (one profile for that resolution), or `sketch.image{profile = \
             name}` names it.",
        ),
        reading(save_scanner_profile),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.image_profile",
            "Read a part on a picture as radius against angle, its peaks counted",
        ))
        .param(
            "x",
            ParamKind::Number,
            "A point on or near the part, in the sketch's coordinates",
        )
        .param("y", ParamKind::Number, "")
        .optional(
            "prominence",
            ParamKind::Number,
            "Peaks standing less than this above their surroundings are not counted, mm; 0.2",
        )
        .optional(
            "samples",
            ParamKind::Bool,
            "Answer the radius at every degree too",
        )
        .returns(
            "{centre = {x, y}, peaks, pitch, outer, inner, at = {angles of the peaks}, radius?}",
        )
        .read_only()
        .note(
            "The part nearest the point (as `sketch.image_objects` finds parts) is read round its \
             centroid: its outline's farthest radius every half degree, and the peaks of that \
             profile standing out by at least `prominence`: a gear's teeth, a knurl's ridges. \
             `pitch` is the mean angle between them, `outer` and `inner` the profile's \
             envelope.",
        )
        .see_also("sketch.image_objects"),
        reading(image_profile),
    );
    t.add_on(
        sketch(CommandSpec::new(
            "sketch.calibrate_image",
            "Scale a picture so a circle on it, at a point of its rim, is a known size across",
        ))
        .param(
            "x",
            ParamKind::Number,
            "A point on the circle's rim, in the sketch's coordinates",
        )
        .param("y", ParamKind::Number, "")
        .param(
            "diameter",
            ParamKind::Number,
            "What the circle measures across, mm",
        )
        .returns("{image, measured, scale}")
        .note(
            "The circle is fitted to the picture's edge round the point, as `sketch.fit_edge` \
             fits it, and the picture scaled about its centre: a gauge pin or a washer of \
             known size laid on the glass beside the part calibrates the scan.",
        )
        .see_also("sketch.fit_edge"),
        |bench, args, ctx| calibrate_image(args, ctx, &bench.tab.picture_edges),
    );
    t.add_free(
        sketch(CommandSpec::new(
            "sketch.image_objects",
            "Find the separate parts on the sketch's pictures, and measure each",
        ))
        .optional("min_area", ParamKind::Number, "Parts smaller than this are left out, mm²; 1")
        .optional("outlines", ParamKind::Bool, "Answer each part's outline too")
        .returns("{{image, area, holes, centroid = {x, y}, diameter, max_width, min_width, long_axis, outline?}}")
        .read_only()
        .note(
            "The picture is split by its grey level into part and ground (the ground is what \
             its border mostly is), and each connected part measured in the sketch's \
             millimetres as the picture lies: its material's area (holes left out), its \
             holes, centroid, the diameter of a disc of its area, its widest and narrowest \
             caliper width, the direction of its long axis from the sketch's x axis in \
             degrees, and with `outlines` its outer boundary.",
        )
        .see_also("sketch.fit_edge"),
        reading(image_objects),
    );
    t.add_on(
        sketch(CommandSpec::new(
            "sketch.fit_edge",
            "Fit a line or a circle to a reference picture's edge where a point lands",
        ))
        .param(
            "x",
            ParamKind::Number,
            "Where, in the sketch's coordinates, mm",
        )
        .param("y", ParamKind::Number, "")
        .optional(
            "kind",
            ParamKind::String,
            "line or circle; the one that runs further when left out",
        )
        .optional(
            "near",
            ParamKind::Number,
            "How far from the point the edge may be, mm; 2",
        )
        .optional(
            "box",
            ParamKind::Any,
            "{{x, y}, {x, y}}: fit every edge point in this box instead of following the edge from \
             the point, a point straying past three times their median stray left out",
        )
        .returns("{element, kind, sigma, points, dropped, length?, radius?}")
        .note(
            "The picture's edges are found once, to a tenth of a pixel, at up to 4096 pixels \
             on its longer side, and drawn over it while the sketch is open. The fit starts \
             from a dozen points on the edge nearest the point and runs on both ways while the \
             next stays within the edge's own noise of the shape, so it stops at a corner; it \
             reports how far its points stray (`sigma`, mm). Without `kind` it takes the shape \
             that runs further. A line on a curved edge is refused, and so is a circle on less \
             than 8° of round.",
        )
        .see_also("sketch.image"),
        |bench, args, ctx| fit_edge(args, ctx, &bench.tab.picture_edges),
    );
}

fn set_image(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let image = a.id("image")?;
    let Some(at) = feature.sketch.images.iter().position(|i| i.id == image) else {
        return Err(CommandError::bad(
            "image",
            "is not a picture of this sketch",
        ));
    };
    if a.opt_bool("remove")?.unwrap_or(false) {
        feature.sketch.images.remove(at);
    } else {
        let placed = place_image(a, feature.sketch.images[at].clone())?;
        feature.sketch.images[at] = placed;
    }
    Ok(Value::Null)
}

fn bolt_circle(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let args = a.0;
    let ids: Vec<Uuid> = match args.get("circles") {
        Some(Value::Array(list)) => list
            .iter()
            .map(|v| {
                v.as_str()
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| CommandError::bad("circles", "are element ids"))
            })
            .collect::<Result<_, _>>()?,
        _ => return Err(CommandError::bad("circles", "lists three or more circles")),
    };
    let centres: Vec<Vec2D> = ids
        .iter()
        .map(|id| match feature.sketch.get_geometry(*id) {
            Some(GeometryElement::Circle(c)) => feature.sketch.point_position(c.center),
            Some(GeometryElement::Arc(a)) => feature.sketch.point_position(a.center),
            _ => None,
        })
        .collect::<Option<_>>()
        .ok_or_else(|| CommandError::bad("circles", "are circles or arcs of the sketch"))?;
    if centres.len() < 3 {
        return Err(CommandError::bad("circles", "lists three or more circles"));
    }
    let Some(crate::picture_edges::Fit::Circle { centre, radius, .. }) =
        crate::picture_edges::circle_fit(&centres)
    else {
        return Err(CommandError::failed("the centres lie on a line"));
    };
    let strays: Vec<f64> = centres
        .iter()
        .map(|p| (p.x - centre.x).hypot(p.y - centre.y) - radius)
        .collect();
    let angles: Vec<f64> = centres
        .iter()
        .map(|p| (p.y - centre.y).atan2(p.x - centre.x).to_degrees())
        .collect();
    let c = point_at(&mut feature.sketch, centre);
    let id = feature
        .sketch
        .add_geometry(GeometryElement::Circle(Circle::new(c, radius)));
    feature.sketch.set_construction(id, true);
    let answer = json!({
        "element": id.to_string(),
        "centre": {"x": centre.x, "y": centre.y},
        "radius": radius,
        "strays": strays,
        "angles": angles,
    });
    Ok(answer)
}

/// The picture the `image` argument numbers (the first when left out):
/// where it stands in the sketch's list, and itself.
fn numbered_image(a: &Args, sketch: &Sketch) -> Result<(usize, ReferenceImage), CommandError> {
    let n = a.opt_number("image")?.unwrap_or(1.0) as usize;
    let k = n
        .checked_sub(1)
        .filter(|k| *k < sketch.images.len())
        .ok_or_else(|| CommandError::bad("image", "names no picture of the sketch"))?;
    Ok((k, sketch.images[k].clone()))
}

/// Picture `k`, `image` until now, laid as `placed`: what the turn came to.
fn lay_image(
    feature: &mut SketchFeature,
    k: usize,
    image: &ReferenceImage,
    placed: ReferenceImage,
) -> Value {
    let answer = json!({
        "image": image.id.to_string(),
        "turned": placed.angle_deg - image.angle_deg,
        "angle": placed.angle_deg,
        "mirrored": placed.mirrored,
    });
    feature.sketch.images[k] = placed;
    answer
}

fn align_image(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let (k, image) = numbered_image(a, &feature.sketch)?;
    let point = |name: &str| a.vec2(name).map(|[x, y]| vec(x, y));
    let placed = crate::images::align(&image, point("origin")?, point("toward")?)
        .ok_or_else(|| CommandError::bad("toward", "must be apart from the origin"))?;
    Ok(lay_image(feature, k, &image, placed))
}

fn turn_image(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let (k, image) = numbered_image(a, &feature.sketch)?;
    let quarters = a
        .opt_number("quarters")?
        .unwrap_or(if a.has("mirror") { 0.0 } else { 1.0 });
    let placed = crate::images::turn(
        &image,
        quarters.round() as i32,
        a.opt_bool("mirror")? == Some(true),
    );
    Ok(lay_image(feature, k, &image, placed))
}

fn save_scanner_profile(
    a: &Args,
    _sketch_id: FeatureId,
    feature: SketchFeature,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let (_, image) = numbered_image(a, &feature.sketch)?;
    let bytes = ctx
        .document
        .asset_bytes(image.asset)
        .ok_or_else(|| CommandError::failed("the picture's file is missing"))?;
    let (w, _) = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_dimensions().ok())
        .ok_or_else(|| CommandError::failed("the picture does not read"))?;
    let x = image.width / f64::from(w.max(1));
    let profile = crate::images::ScannerProfile {
        name: a.string("name")?.to_string(),
        dpi: image.dpi,
        mm_per_px: [x, x * image.stretch],
    };
    crate::images::keep_scanner_profile(profile.clone()).map_err(CommandError::failed)?;
    Ok(json!({"name": profile.name, "dpi": profile.dpi,
              "mm_per_px": {"x": profile.mm_per_px[0], "y": profile.mm_per_px[1]}}))
}

fn calibrate_axis(
    a: &Args,
    _sketch_id: FeatureId,
    feature: &mut SketchFeature,
    _ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let (k, image) = numbered_image(a, &feature.sketch)?;
    let point = |name: &str| a.vec2(name).map(|[x, y]| vec(x, y));
    let y = match a.string("axis")? {
        "x" => false,
        "y" => true,
        _ => return Err(CommandError::bad("axis", "is x or y")),
    };
    let placed =
        crate::images::calibrate_axis(&image, point("from")?, point("to")?, a.number("length")?, y)
            .ok_or_else(|| {
                CommandError::failed(
                    "the points are not apart along that axis, or the length is not a size",
                )
            })?;
    let scale = if y {
        placed.stretch / image.stretch
    } else {
        placed.width / image.width
    };
    feature.sketch.images[k] = placed;
    Ok(json!({"image": image.id.to_string(), "scale": scale}))
}

fn image_profile(
    a: &Args,
    _sketch_id: FeatureId,
    feature: SketchFeature,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let at = vec(a.number("x")?, a.number("y")?);
    let prominence = a.opt_number("prominence")?.unwrap_or(0.2);
    let mut best: Option<(f64, Vec2D, Vec<[f64; 2]>)> = None;
    for image in &feature.sketch.images {
        let Some(bytes) = ctx.document.asset_bytes(image.asset) else {
            continue;
        };
        let Some((w, h, found)) = crate::picture_edges::objects_of(bytes, 1.0, image.stretch)
        else {
            continue;
        };
        let set = crate::picture_edges::EdgeSet {
            width: w,
            height: h,
            chains: Vec::new(),
        };
        for o in found {
            let c = crate::picture_edges::to_sketch(image, &set, o.centroid);
            let d = (c.x - at.x).hypot(c.y - at.y);
            if best.as_ref().is_none_or(|(b, _, _)| d < *b) {
                let outline = o
                    .outline
                    .iter()
                    .map(|p| {
                        let q = crate::picture_edges::to_sketch(image, &set, *p);
                        [q.x, q.y]
                    })
                    .collect();
                best = Some((d, c, outline));
            }
        }
    }
    let (_, centre, outline) =
        best.ok_or_else(|| CommandError::failed("no part on the sketch's pictures"))?;
    let radius = metrology::profile::radial(&outline, [centre.x, centre.y], 720);
    let found = metrology::profile::peaks(&radius, prominence);
    let (inner, outer) = radius
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), r| (a.min(*r), b.max(*r)));
    let mut answer = json!({
        "centre": {"x": centre.x, "y": centre.y},
        "peaks": found.len(),
        "pitch": if found.is_empty() { Value::Null } else { json!(360.0 / found.len() as f64) },
        "outer": outer,
        "inner": inner,
        "at": found.iter().map(|p| p.at as f64 / 2.0).collect::<Vec<_>>(),
    });
    if a.opt_bool("samples")? == Some(true) {
        answer["radius"] = json!(radius.iter().step_by(2).collect::<Vec<_>>());
    }
    Ok(answer)
}

fn image_objects(
    a: &Args,
    _sketch_id: FeatureId,
    feature: SketchFeature,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let min_area = a.opt_number("min_area")?.unwrap_or(1.0);
    let outlines = a.opt_bool("outlines")? == Some(true);
    let mut out = Vec::new();
    for (n, image) in feature.sketch.images.iter().enumerate() {
        let Some(bytes) = ctx.document.asset_bytes(image.asset) else {
            continue;
        };
        let Some((w, h, found)) = crate::picture_edges::objects_of(bytes, 1.0, image.stretch)
        else {
            continue;
        };
        let set = crate::picture_edges::EdgeSet {
            width: w,
            height: h,
            chains: Vec::new(),
        };
        let mm = image.width / w.max(1) as f64;
        for o in found {
            let area = o.area * mm * mm;
            if area < min_area {
                continue;
            }
            let c = crate::picture_edges::to_sketch(image, &set, o.centroid);
            let mut axis = o.long_axis_deg + image.angle_deg;
            axis = (axis + 90.0).rem_euclid(180.0) - 90.0;
            let mut entry = json!({
                "image": n + 1,
                "area": area,
                "holes": o.holes,
                "centroid": {"x": c.x, "y": c.y},
                "diameter": o.equivalent_diameter * mm,
                "max_width": o.max_width * mm,
                "min_width": o.min_width * mm,
                "long_axis": axis,
            });
            if outlines {
                entry["outline"] = json!(
                    o.outline
                        .iter()
                        .map(|p| {
                            let q = crate::picture_edges::to_sketch(image, &set, *p);
                            [q.x, q.y]
                        })
                        .collect::<Vec<_>>()
                );
            }
            out.push(entry);
        }
    }
    if feature.sketch.images.is_empty() {
        return Err(CommandError::failed("the sketch has no picture"));
    }
    Ok(json!(out))
}

/// `spec` taking where a picture lies: its middle, width, turn and opacity.
fn placed(spec: CommandSpec) -> CommandSpec {
    spec.optional("x", ParamKind::Number, "Its middle, mm; 0 when left out")
        .optional("y", ParamKind::Number, "Its middle, mm; 0 when left out")
        .optional("width", ParamKind::Number, "How wide it lies, mm")
        .optional(
            "angle",
            ParamKind::Number,
            "Its turn counter-clockwise, degrees",
        )
        .optional("opacity", ParamKind::Number, "How much of it shows, 0 to 1")
}

/// A picture's placement from `a`, over `image`'s where `a` says nothing.
fn place_image(
    a: &Args,
    mut image: crate::sketch::ReferenceImage,
) -> Result<crate::sketch::ReferenceImage, CommandError> {
    if let Some(x) = a.opt_number("x")? {
        image.center.x = x;
    }
    if let Some(y) = a.opt_number("y")? {
        image.center.y = y;
    }
    if let Some(width) = a.opt_number("width")? {
        if !(width > 0.0 && width.is_finite()) {
            return Err(CommandError::bad("width", "must be more than zero"));
        }
        image.width = width;
    }
    if let Some(angle) = a.opt_number("angle")? {
        image.angle_deg = angle;
    }
    if let Some(opacity) = a.opt_number("opacity")? {
        image.opacity = opacity.clamp(0.0, 1.0);
    }
    Ok(image)
}

/// Lay a picture file on a sketch's plane: the file kept in the document,
/// the picture in the sketch given, else the one being edited, else a new
/// sketch named after the file.
fn add_image(a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let path = std::path::Path::new(a.string("path")?);
    let bytes = std::fs::read(path)
        .map_err(|e| CommandError::bad("path", format!("could not be read: {e}")))?;
    crate::images::decode(&bytes)
        .map_err(|e| CommandError::bad("path", format!("is not a picture printCAD reads: {e}")))?;
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "png".to_string());
    // Without a width, the size the file's resolution says.
    let stated = crate::images::dpi(&bytes);
    let pixels_wide = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_dimensions().ok())
        .map(|(w, _)| f64::from(w));
    let nominal = match (a.has("width"), stated, pixels_wide) {
        (false, Some(dpi), Some(w)) => Some((dpi, w / dpi * 25.4)),
        _ => None,
    };
    let asset = ctx.document.add_asset_with_data(
        core_document::AssetReference::new(
            format!("assets/reference-{}.{extension}", Uuid::new_v4()),
            core_document::AssetType::Other,
            json!({"kind": "reference image", "file": path.file_name().map(|n| n.to_string_lossy())}),
        ),
        bytes,
    );
    let mut image = place_image(
        a,
        crate::sketch::ReferenceImage {
            id: Uuid::new_v4(),
            asset,
            center: Vec2D::new(0.0, 0.0),
            width: DEFAULT_IMAGE_WIDTH_MM,
            angle_deg: 0.0,
            opacity: 0.5,
            scale: Default::default(),
            stretch: 1.0,
            dpi: None,
            mirrored: false,
        },
    )?;
    image.dpi = stated;
    if let Some((dpi, width)) = nominal {
        image.width = width;
        image.scale = crate::sketch::ImageScale::Nominal { dpi };
    }
    // A scanner profile named, else the one kept for the file's resolution.
    let profiles = crate::images::scanner_profiles();
    let profile = match a.opt_string("profile")? {
        Some(name) => Some(profiles.iter().find(|p| p.name == name).ok_or_else(|| {
            CommandError::bad("profile", format!("no scanner profile is named {name}"))
        })?),
        None if !a.has("width") => {
            let fits: Vec<_> = profiles
                .iter()
                .filter(|p| p.dpi.zip(stated).is_some_and(|(a, b)| (a - b).abs() < 0.5))
                .collect();
            (fits.len() == 1).then(|| fits[0])
        }
        None => None,
    };
    if let (Some(profile), Some(w)) = (profile, pixels_wide) {
        image = crate::images::apply_profile(&image, w, profile);
    }
    let is_sketch = |id: FeatureId| {
        ctx.document
            .get_feature_meta(id)
            .is_some_and(|n| n.workbench_id.as_str() == crate::KIND)
    };
    let target = match a.opt_id("sketch")? {
        Some(id) => Some(FeatureId(id)),
        None => ctx.active_document_object.filter(|id| is_sketch(*id)),
    };
    let answer =
        |sketch: FeatureId| json!({"sketch": sketch.0.to_string(), "image": image.id.to_string()});
    match target {
        Some(id) => {
            let mut feature = load(ctx, id)?;
            feature.sketch.images.push(image.clone());
            save(ctx, id, feature, answer(id))
        }
        None => {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| crate::SketchWorkbench::next_sketch_name(ctx.document));
            let mut sketch = Sketch::new(name);
            sketch.images.push(image.clone());
            let id = add_sketch(a, ctx, sketch, None)?;
            Ok(answer(id))
        }
    }
}

/// How wide a picture lies when nothing says, mm.
const DEFAULT_IMAGE_WIDTH_MM: f64 = 100.0;

/// `sketch.calibrate_image`: the picture under a point scaled so the
/// circle whose rim it is is `diameter` across.
pub(crate) fn calibrate_image(
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
    edges: &crate::picture_edges::Edges,
) -> CommandResult {
    use crate::picture_edges::{Fit, fit};
    let a = Args(args);
    let sketch_id = FeatureId(a.id("sketch")?);
    let mut feature = load(ctx, sketch_id)?;
    let at = vec(a.number("x")?, a.number("y")?);
    let known = a.number("diameter")?;
    let mut why = String::from("no circle's edge there");
    for k in 0..feature.sketch.images.len() {
        let image = feature.sketch.images[k].clone();
        let Some(set) = edges.now(image.asset, || {
            ctx.document.asset_bytes(image.asset).map(<[u8]>::to_vec)
        }) else {
            continue;
        };
        match fit(&image, &set, at, 2.0, Some(true)) {
            Ok(Fit::Circle { centre, radius, .. }) => {
                let placed =
                    crate::images::calibrate_by_circle(&image, centre, 2.0 * radius, known)
                        .ok_or_else(|| CommandError::bad("diameter", "must be more than zero"))?;
                let scale = placed.width / image.width;
                let id = image.id;
                feature.sketch.images[k] = placed;
                return save(
                    ctx,
                    sketch_id,
                    feature,
                    json!({"image": id.to_string(), "measured": 2.0 * radius, "scale": scale}),
                );
            }
            Ok(_) => {}
            Err(e) => why = e,
        }
    }
    Err(CommandError::failed(why))
}

/// `sketch.fit_edge`: a line or circle fitted to the edge of a reference
/// picture of the sketch nearest a point.
pub(crate) fn fit_edge(
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
    edges: &crate::picture_edges::Edges,
) -> CommandResult {
    use crate::picture_edges::{Fit, fit};
    let a = Args(args);
    let sketch_id = FeatureId(a.id("sketch")?);
    let mut feature = load(ctx, sketch_id)?;
    let at = vec(a.number("x")?, a.number("y")?);
    let near = a.opt_number("near")?.unwrap_or(2.0);
    let want = match a.opt_string("kind")? {
        None => None,
        Some("line") => Some(false),
        Some("circle") => Some(true),
        Some(_) => return Err(CommandError::bad("kind", "is line or circle")),
    };
    if feature.sketch.images.is_empty() {
        return Err(CommandError::failed("the sketch has no picture"));
    }
    let region = match args.get("box") {
        Some(Value::Array(corners)) if corners.len() == 2 => {
            let c = |v: &Value| vec2_of(Some(v), "box").map(|[x, y]| vec(x, y));
            Some((c(&corners[0])?, c(&corners[1])?))
        }
        Some(_) => return Err(CommandError::bad("box", "is two {x, y} corners")),
        None => None,
    };
    let mut why = String::from("no edge there");
    let mut found = None;
    let mut dropped = 0;
    for image in &feature.sketch.images {
        let Some(set) = edges.now(image.asset, || {
            ctx.document.asset_bytes(image.asset).map(<[u8]>::to_vec)
        }) else {
            continue;
        };
        let fitted = match region {
            Some((lo, hi)) => {
                crate::picture_edges::fit_region(image, &set, lo, hi, want).map(|(f, d)| {
                    dropped = d;
                    f
                })
            }
            None => fit(image, &set, at, near, want),
        };
        match fitted {
            Ok(f) => {
                found = Some(f);
                break;
            }
            Err(e) => why = e,
        }
    }
    let fitted = found.ok_or_else(|| CommandError::failed(why))?;
    let sketch = &mut feature.sketch;
    let answer = match fitted {
        Fit::Line {
            from,
            to,
            sigma,
            points,
        } => {
            let id = line(sketch, from, to)?;
            json!({"element": id.to_string(), "kind": "line", "sigma": sigma, "points": points,
                   "dropped": dropped, "length": (to.x - from.x).hypot(to.y - from.y)})
        }
        Fit::Circle {
            centre,
            radius,
            sigma,
            points,
        } => {
            let c = point_at(sketch, centre);
            let id = sketch.add_geometry(GeometryElement::Circle(Circle::new(c, radius)));
            json!({"element": id.to_string(), "kind": "circle", "sigma": sigma, "points": points,
                   "dropped": dropped, "radius": radius})
        }
    };
    save(ctx, sketch_id, feature, answer)
}
