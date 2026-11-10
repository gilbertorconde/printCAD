//! Elements and what is made from them: comparisons, fitted, grown and
//! constructed elements, dimensions, and the report.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, ParamKind,
    WorkbenchContext, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};

use crate::{
    COMPARE_COMMAND, CONSTRUCT_COMMAND, Comparison, DEFAULT_CUTOFF, DIMENSION_COMMAND, DimKind,
    Element, FIT_COMMAND, GROW_COMMAND, InspectionWorkbench, Made, REPORT_COMMAND, add_comparison,
    add_construction, add_dimension, answer_of, dimension, element_at, fit_marked, grow_at, ids_in,
    kind_named, method_arg, report, report_of,
};

pub(super) fn owns(id: &str) -> bool {
    matches!(
        id,
        COMPARE_COMMAND
            | CONSTRUCT_COMMAND
            | FIT_COMMAND
            | DIMENSION_COMMAND
            | GROW_COMMAND
            | REPORT_COMMAND
    )
}

pub(super) fn configure(context: &mut WorkbenchContext) {
    context.register_command(
            CommandSpec::new(
                COMPARE_COMMAND,
                "Compare a scan with a reference body, point by point",
            )
            .param("scan", ParamKind::Id, "The scan")
            .param("reference", ParamKind::Id, "The body it should be: a solid or a mesh")
            .optional("tolerance", ParamKind::Number, "± in mm: the share within it is counted")
            .optional("range", ParamKind::Number, "The colour scale's half-width in mm; worked out when left out")
            .optional("bands", ParamKind::Integer, "Steps of flat colour; 0, continuous, when left out")
            .optional("ramp", ParamKind::String, "jet, viridis, magma, inferno or plasma")
            .optional("max", ParamKind::Number, "Leave points farther than this from the reference unmeasured")
            .optional("facing", ParamKind::Number, "Measure only to reference surface within this many degrees of a point's own normal; 90 when left out")
            .returns("{comparison, name, scan, reference, range, measured, unmeasured, mean, rms, sigma, min, max, within}")
            .note(
                "A distance is positive where the scan stands outside the reference, negative \
                 inside. A solid reference is measured exactly as it is meshed for the purpose, \
                 finely; a mesh one as its triangles. Both are measured where they sit, so align \
                 the scan first. The map is drawn on the scan while the Inspection bench is \
                 active.",
            ),
        );
    context.register_command(
        CommandSpec::new(
            CONSTRUCT_COMMAND,
            "Make a point, line, plane or circle from elements",
        )
        .param("made", ParamKind::String, "point, line, plane or circle")
        .param(
            "from",
            ParamKind::List,
            "The ids of the elements it is made from, fitted or constructed",
        )
        .optional(
            "offset",
            ParamKind::Number,
            "A plane made from one plane lies this far along its normal; 0 when left out",
        )
        .returns("{element, name, made, from, offset, shape}: the shape in the scan's frame")
        .note(
            "A point is two centres' midpoint or where an axis crosses a plane; a line runs \
                 through two centres or where two planes meet; a plane runs through three \
                 centres, midway between two planes or offset from one; a circle is where a \
                 plane cuts a sphere, or a cylinder within 5° of square. A sphere, circle or \
                 torus gives its centre, a cylinder, cone, circle or line its axis. The \
                 construction follows its sources when they are refitted, and dimensions take \
                 it as they take a fitted element.",
        ),
    );
    context.register_command(
        CommandSpec::new(
            FIT_COMMAND,
            "Fit a shape to the marked part of a scan and keep it as an element",
        )
        .param(
            "kind",
            ParamKind::String,
            "plane, sphere, cylinder, cone, torus, circle or line",
        )
        .optional(
            "cutoff",
            ParamKind::Number,
            "Leave out points farther than this many sigmas, fitting again until the \
                 set kept stops changing; 3 when left out, 0 keeps every point",
        )
        .optional(
            "method",
            ParamKind::String,
            "least_squares (the default), minimum_zone, inscribed or circumscribed",
        )
        .returns(
            "{element, name, kind, shape, sigma, form, zone, method, diameters, used, \
                 points}: in mm, the shape in the scan's own frame",
        )
        .note(
            "It fits what is marked (`pc.scan.mark`, or Mark surface… in the scan's \
                 menu), by orthogonal least squares. `sigma` is the RMS distance of the points \
                 used from the shape, `form` the width of the band they fill, `diameters` the \
                 smallest and largest a round shape reaches.",
        )
        .note(
            "Least squares picks the points used; `method` then fits those another way: \
                 `minimum_zone` to the middle of the narrowest zone holding them (a plane, \
                 line, circle, cylinder or sphere), `inscribed` to the largest circle or \
                 cylinder clear of them (a hole's mating size), `circumscribed` to the \
                 smallest round them (a boss's). `zone` is the minimum zone's width of the \
                 points used, {width, optimal}, the form a flatness, straightness, \
                 circularity, cylindricity or sphericity tolerance reads; `optimal` is false \
                 where the zone of a line, cylinder or sphere is only certified locally.",
        ),
    );
    context.register_command(
        CommandSpec::new(
            DIMENSION_COMMAND,
            "Measure a distance, angle or diameter between elements",
        )
        .param("kind", ParamKind::String, "distance, angle or diameter")
        .param(
            "elements",
            ParamKind::List,
            "The elements' ids: two for a distance or an angle, one for a diameter",
        )
        .optional(
            "nominal",
            ParamKind::Number,
            "What it should read; the nearest design value when left out",
        )
        .optional(
            "plus",
            ParamKind::Number,
            "How far above the nominal it may read",
        )
        .optional(
            "minus",
            ParamKind::Number,
            "How far below the nominal it may read",
        )
        .returns(
            "{dimension, name, kind, elements, nominal, plus, minus, value, deviation, \
                 deltas, range, pass}",
        )
        .note(
            "A distance is measured from the middle of the first element: between centres \
                 (spheres, circles) with `deltas` along X, Y and Z; from a centre to an axis \
                 or a plane; between near-parallel axes as their offset, otherwise where they \
                 come closest; between planes or an axis and a plane only within 3° of \
                 parallel, planes with the `range` their surfaces give. An angle between axes \
                 or an axis and a plane reads 0–90°, between planes' normals 0–180°. \
                 `pass` is there once `plus` or `minus` is above zero. A dimension reads its \
                 elements again whenever they change; `doc.set_value` sets its nominal, plus \
                 and minus.",
        ),
    );
    context.register_command(
        CommandSpec::new(
            GROW_COMMAND,
            "Fit a surface grown, or an edge traced, from a point on a scan",
        )
        .param(
            "kind",
            ParamKind::String,
            "plane, sphere, cylinder, cone or torus",
        )
        .param("scan", ParamKind::Id, "The scan")
        .param(
            "at",
            ParamKind::Any,
            "{x, y, z}: a point on or near the surface, in the scan's own frame",
        )
        .optional(
            "cutoff",
            ParamKind::Number,
            "Points farther than this many sigmas stop the growth and are left out; 3 \
                 when left out",
        )
        .optional(
            "more",
            ParamKind::List,
            "More {x, y, z} points in the scan's frame the region grows from as well \
                 (the clicks pinned with Shift before the last)",
        )
        .optional(
            "peel",
            ParamKind::Bool,
            "Take off the region's rim while it turns away (a face's rounds)",
        )
        .optional(
            "method",
            ParamKind::String,
            "least_squares (the default), minimum_zone, inscribed or circumscribed",
        )
        .returns("as inspect.fit")
        .note(
            "The region starts at the triangle nearest `at` and takes each neighbour whose \
                 points lie on the fitted surface and whose normal agrees with it, fitting \
                 again as it grows. A circle or a line is traced instead along the sharp edge \
                 nearest `at` (a hole's rim, a box's edge) until the edge turns away or meets \
                 another. It is refused where what is at `at` is not of the kind asked.",
        )
        .example(
            "A scanned bore's mating size sits inside its mean size",
            r#"
                    local drum = pc.doc.new_body{}
                    pc.design.primitive{body = drum, variant = "cylinder",
                      kind = {Cylinder = {radius = 5, height = 10, angle_deg = 360}}}
                    assert(#pc.doc.rebuild() == 0)
                    local path = os.tmpname() .. ".stl"
                    pc.file.export{path = path, bodies = {drum}}
                    pc.doc.set_visible{id = drum, visible = false}
                    pc.file.import{path = path, scan = true}
                    pc.doc.rebuild()
                    os.remove(path)
                    local scan
                    for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
                    local mean = pc.inspect.grow{kind = "cylinder", scan = scan, at = {5, 0, 5}}
                    local inside = pc.inspect.grow{kind = "cylinder", scan = scan,
                      at = {5, 0, 5}, method = "inscribed"}
                    assert(inside.method == "inscribed")
                    assert(inside.shape.Cylinder.radius <= mean.shape.Cylinder.radius + 1e-9)
                    assert(mean.zone.width <= mean.form + 1e-9, mean.zone.width)
                    "#,
        ),
    );
    context.register_command(
        CommandSpec::new(
            REPORT_COMMAND,
            "Every element and dimension of the inspection",
        )
        .optional("scan", ParamKind::Id, "Only this scan's")
        .optional(
            "format",
            ParamKind::String,
            "json (the answer as a table), csv or text",
        )
        .optional(
            "path",
            ParamKind::String,
            "Write it to this file; its extension picks the format when `format` is left out",
        )
        .returns(
            "{elements, constructions, dimensions, tolerances, passed, failed}, each as \
                 the command making it answers; CSV or text as a string; {path} when written",
        )
        .read_only(),
    );
}

pub(super) fn run(
    bench: &InspectionWorkbench,
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    match id {
        COMPARE_COMMAND => {
            let c = Comparison {
                name: String::new(),
                scan: BodyId(a.id("scan")?),
                reference: BodyId(a.id("reference")?),
                range: a.opt_number("range")?,
                bands: a.opt_number("bands")?.map_or(0, |b| b.max(0.0) as u32),
                ramp: a
                    .opt_string("ramp")?
                    .unwrap_or(metrology::ramp::Ramp::Jet.name())
                    .to_string(),
                tolerance: a.opt_number("tolerance")?,
                max: a.opt_number("max")?,
                facing_limit_deg: Some(a.opt_number("facing")?.unwrap_or(90.0)),
                volume_cell: None,
            };
            let (_, answer) = add_comparison(ctx, &bench.maps, c).map_err(CommandError::failed)?;
            Ok(answer)
        }
        CONSTRUCT_COMMAND => {
            let made = Made::ALL
                .into_iter()
                .find(|m| {
                    m.word()
                        .eq_ignore_ascii_case(a.string("made").unwrap_or(""))
                })
                .ok_or_else(|| CommandError::bad("made", "is point, line, plane or circle"))?;
            let from = ids_in(&a, "from")?;
            let offset = a.opt_number("offset")?.unwrap_or(0.0);
            let (_, answer) =
                add_construction(ctx, made, from, offset).map_err(CommandError::failed)?;
            Ok(answer)
        }
        FIT_COMMAND => {
            let kind = kind_named(a.string("kind")?).ok_or_else(|| {
                CommandError::bad(
                    "kind",
                    "is plane, sphere, cylinder, cone, torus, circle or line",
                )
            })?;
            let cutoff = match a.opt_number("cutoff")? {
                Some(k) if k <= 0.0 => None,
                Some(k) => Some(k),
                None => Some(DEFAULT_CUTOFF),
            };
            let (id, element) =
                fit_marked(ctx, kind, cutoff, method_arg(&a)?).map_err(CommandError::failed)?;
            Ok(answer_of(id, &element))
        }
        DIMENSION_COMMAND => {
            let kind = DimKind::ALL
                .into_iter()
                .find(|k| {
                    k.word()
                        .eq_ignore_ascii_case(a.string("kind").unwrap_or(""))
                })
                .ok_or_else(|| CommandError::bad("kind", "is distance, angle or diameter"))?;
            let elements = ids_in(&a, "elements")?;
            let band = match (
                a.opt_number("nominal")?,
                a.opt_number("plus")?,
                a.opt_number("minus")?,
            ) {
                (None, None, None) => None,
                (nominal, plus, minus) => {
                    let found: Vec<Element> = elements
                        .iter()
                        .filter_map(|id| element_at(ctx.document, *id))
                        .collect();
                    let nominal = match nominal {
                        Some(n) => n,
                        None => dimension::design_value(
                            kind,
                            dimension::read(kind, &found)
                                .map_err(CommandError::failed)?
                                .value,
                        ),
                    };
                    Some((
                        nominal,
                        plus.unwrap_or(0.0).abs(),
                        minus.unwrap_or(0.0).abs(),
                    ))
                }
            };
            let (_, answer) =
                add_dimension(ctx, kind, elements, band).map_err(CommandError::failed)?;
            Ok(answer)
        }
        GROW_COMMAND => {
            let kind = kind_named(a.string("kind")?).ok_or_else(|| {
                CommandError::bad(
                    "kind",
                    "is plane, sphere, cylinder, cone, torus, circle or line",
                )
            })?;
            let scan = BodyId(a.id("scan")?);
            let at = a.vec3("at")?;
            let cutoff = match a.opt_number("cutoff")? {
                Some(k) if k <= 0.0 => None,
                Some(k) => Some(k),
                None => Some(DEFAULT_CUTOFF),
            };
            let peel = a.opt_bool("peel")? == Some(true);
            let more = match a.0.get("more") {
                Some(Value::Array(list)) => list
                    .iter()
                    .map(|p| core_document::command::vec3_of(Some(p), "more"))
                    .collect::<Result<Vec<_>, _>>()?,
                _ => Vec::new(),
            };
            let (id, element) = grow_at(ctx, kind, scan, at, &more, cutoff, peel, method_arg(&a)?)
                .map_err(CommandError::failed)?;
            Ok(answer_of(id, &element))
        }
        REPORT_COMMAND => {
            let only = a.opt_id("scan")?.map(BodyId);
            let report = report_of(
                ctx.document,
                only,
                &bench.tolerance_readings,
                ctx.kernel,
                true,
            );
            let path = a.opt_string("path")?.map(std::path::PathBuf::from);
            let format = match a.opt_string("format")? {
                Some(f) => f.to_lowercase(),
                None => path
                    .as_ref()
                    .and_then(|p| p.extension()?.to_str().map(str::to_lowercase))
                    .unwrap_or_else(|| "json".to_string()),
            };
            let written = match format.as_str() {
                "json" => serde_json::to_string_pretty(&report).unwrap_or_default(),
                "csv" => report::csv(&report),
                "text" | "txt" => report::text(&report, "Inspection report"),
                _ => return Err(CommandError::bad("format", "is json, csv or text")),
            };
            match path {
                Some(path) => {
                    std::fs::write(&path, written)
                        .map_err(|e| CommandError::failed(format!("{}: {e}", path.display())))?;
                    Ok(json!({"path": path.display().to_string()}))
                }
                None if format == "json" => Ok(report),
                None => Ok(json!(written)),
            }
        }
        _ => Err(CommandError::Unknown(id.to_string())),
    }
}
