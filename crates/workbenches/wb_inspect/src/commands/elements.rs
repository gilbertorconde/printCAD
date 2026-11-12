//! Elements and what is made from them: comparisons, fitted, grown and
//! constructed elements, dimensions, and the report.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, ParamKind,
    WorkbenchRuntimeContext,
};
use serde_json::{Value, json};

use crate::{
    COMPARE_COMMAND, CONSTRUCT_COMMAND, Comparison, DEFAULT_CUTOFF, DIMENSION_COMMAND, DimKind,
    Element, FIT_COMMAND, GROW_COMMAND, InspectionWorkbench, Made, REPORT_COMMAND, add_comparison,
    add_construction, add_dimension, answer_of, dimension, element_at, fit_marked, grow_at, ids_in,
    kind_named, method_arg, report, report_of,
};

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<InspectionWorkbench>) {
    t.add_on(
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
            .returns("{comparison, name, scan, reference, range, measured, unmeasured, mean, rms, sigma, min, max, within}"),
        compare,
    );
    t.add(
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
        .returns("{element, name, made, from, offset, shape}: the shape in the scan's frame"),
        construct,
    );
    t.add(
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
        ),
        fit,
    );
    t.add(
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
        ),
        dimension_command,
    );
    t.add(
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
        .returns("as inspect.fit"),
        grow,
    );
    t.add_on(
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
        report_command,
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "elements.md", include_str!("elements.md"), &[])
}

fn compare(
    bench: &mut InspectionWorkbench,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
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
    let (_, answer) = add_comparison(ctx, &bench.tab.maps, c).map_err(CommandError::failed)?;
    Ok(answer)
}

fn construct(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let made = Made::ALL
        .into_iter()
        .find(|m| {
            m.word()
                .eq_ignore_ascii_case(a.string("made").unwrap_or(""))
        })
        .ok_or_else(|| CommandError::bad("made", "is point, line, plane or circle"))?;
    let from = ids_in(&a, "from")?;
    let offset = a.opt_number("offset")?.unwrap_or(0.0);
    let (_, answer) = add_construction(ctx, made, from, offset).map_err(CommandError::failed)?;
    Ok(answer)
}

fn fit(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
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

fn dimension_command(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
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
    let (_, answer) = add_dimension(ctx, kind, elements, band).map_err(CommandError::failed)?;
    Ok(answer)
}

fn grow(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
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

fn report_command(
    bench: &mut InspectionWorkbench,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    let only = a.opt_id("scan")?.map(BodyId);
    let report = report_of(
        ctx.document,
        only,
        &bench.tab.tolerance_readings,
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
