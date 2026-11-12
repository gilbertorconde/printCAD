//! Geometric tolerances read off elements.

use core_document::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, ParamKind,
    WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::json;

use crate::{
    InspectionWorkbench, TOLERANCE, add_tolerance, element_at, tolerance, tolerance_answer,
    tolerance_read,
};

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<InspectionWorkbench>) {
    t.add(
            CommandSpec::new(
                TOLERANCE,
                "Keep a geometric tolerance on an element",
            )
            .param(
                "characteristic",
                ParamKind::String,
                "flatness, straightness, circularity, cylindricity, sphericity, \
                     parallelism, perpendicularity, angularity, coaxiality, concentricity, position, \
                     surface_profile, line_profile, circular_runout, total_runout or symmetry",
            )
            .param("element", ParamKind::Id, "The element toleranced")
            .optional("datum", ParamKind::Id, "The element it is taken against")
            .param(
                "limit",
                ParamKind::Number,
                "The widest zone that passes, mm",
            )
            .optional(
                "angle",
                ParamKind::Number,
                "Angularity's basic angle, degrees",
            )
            .optional("tolerance", ParamKind::Id, "Replace this tolerance rather than creating one")
            .optional("datums", ParamKind::List, "Ordered {element, modifier = mmc or lmc, size} references, up to three; legacy datum stays primary")
            .optional("basic", ParamKind::List, "True position {x, y, z} in the datum frame, mm")
            .optional("basic_direction", ParamKind::List, "Zone axis or plane normal in the datum frame; {0, 0, 1}")
            .optional("material", ParamKind::String, "mmc or lmc on position")
            .optional("material_size", ParamKind::Number, "Material boundary size in mm; otherwise the diameter dimension supplies it")
            .optional("unequal", ParamKind::Number, "Unequally disposed profile zone: signed offset of its centre, mm")
            .optional("section", ParamKind::Id, "Section whose chains a line profile measures")
            .optional("comparison", ParamKind::Id, "Comparison supplying the nominal reference body")
            .optional("reference_face", ParamKind::Integer, "Face index in the reference supplying the basic nominal (plane, cylinder or sphere)")
            .optional("opposed", ParamKind::Id, "Other plane of a slot or symmetry measurement")
            .optional("section_width", ParamKind::Number, "Circular runout station-band width in mm")
            .returns("{tolerance, name, characteristic, element, datum, datums, limit, value, bonus, allowed, datum_shift, pass, note}"),
        tolerance_command,
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "tolerances.md", include_str!("tolerances.md"), &[])
}

fn tolerance_command(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let characteristic = tolerance::Characteristic::ALL
        .into_iter()
        .find(|c| {
            c.key()
                .eq_ignore_ascii_case(&a.string("characteristic").unwrap_or("").replace(' ', "_"))
        })
        .ok_or_else(|| CommandError::bad("characteristic", "is not a tolerance characteristic"))?;
    let datum = a.opt_id("datum")?.map(core_document::FeatureId);
    let fields = serde_json::to_value(args).map_err(|e| CommandError::failed(e.to_string()))?;
    let mut t: tolerance::Tolerance = serde_json::from_value(json!({
        "name": "", "scan": uuid::Uuid::nil(), "characteristic": characteristic,
        "element": a.id("element")?, "datum": datum, "limit": a.number("limit")?,
    }))
    .map_err(|e| CommandError::failed(e.to_string()))?;
    let mut data = t.to_json();
    for key in [
        "datums",
        "basic",
        "basic_direction",
        "material",
        "material_size",
        "unequal",
        "section",
        "comparison",
        "reference_face",
        "opposed",
        "section_width",
    ] {
        if let Some(v) = fields.get(key) {
            data[key] = v.clone();
        }
    }
    t = tolerance::Tolerance::from_json(&data).map_err(|e| CommandError::failed(e.to_string()))?;
    t.angle_deg = a.opt_number("angle")?;
    if let Some(id) = a.opt_id("tolerance")? {
        let id = core_document::FeatureId(id);
        let old = ctx
            .document
            .feature_tree()
            .get_node(id)
            .ok_or_else(|| CommandError::bad("tolerance", "is gone"))?;
        if old.workbench_id.as_str() != tolerance::KIND {
            return Err(CommandError::bad("tolerance", "is not a tolerance"));
        }
        let body = old.body;
        t.name = old.name.clone();
        t.scan = element_at(ctx.document, t.element)
            .ok_or_else(|| CommandError::bad("element", "is gone"))?
            .scan;
        if body != Some(t.scan) {
            return Err(CommandError::bad(
                "element",
                "must stay on the tolerance's scan",
            ));
        }
        let reading = tolerance_read::evaluate(
            ctx.document,
            &t,
            ctx.kernel.map(|k| k as &dyn kernel_api::KernelQueries),
        )
        .map_err(CommandError::failed)?;
        ctx.document
            .update_feature_data(id, t.to_json())
            .map_err(|e| CommandError::failed(e.to_string()))?;
        ctx.document.set_feature_dependencies(id, t.dependencies());
        return Ok(tolerance_answer(id, &t, &Ok(reading)));
    }
    add_tolerance(ctx, t).map_err(CommandError::failed)
}
