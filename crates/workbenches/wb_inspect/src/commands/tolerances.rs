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
            .returns("{tolerance, name, characteristic, element, datum, datums, limit, value, bonus, allowed, datum_shift, pass, note}")
            .note(
                "A form tolerance reads the element's own band: a plane's flatness, a \
                     line's straightness, a circle's circularity, a cylinder's cylindricity, a \
                     sphere's sphericity. An orientation tolerance reads a plane at its measured \
                     points (a constructed plane at its drawn corners, said in `note`), an axis \
                     over its fitted length and a centre as a point, against the datum's \
                     attitude. Form uses the fitted minimum zone when available. Position reads \
                     axis ends or a centre in a cylindrical zone (diameter), a plane in two \
                     parallel planes. The primary datum sets Z, the next transverse datum X. \
                     Basic offsets and nominal faces use that frame. Profiles with datums hold \
                     the scan in this frame against a reference in matching coordinates; without \
                     datums they retain the current best-fit placement. The profile limit is the \
                     full zone width. Circular runout reads sampled station bands; total reads \
                     the whole region, removing a cone's nominal slope. Position bonus is the \
                     departure of the fitted size from material_size; use an inscribed fit for \
                     holes or circumscribed for bosses. Datum shifts are reported, never applied.",
            )
            .example("Datum position, profiles, runout and a median plane", r#"
                local model = pc.doc.new_body{}
                pc.design.primitive{body = model, variant = "cylinder",
                  kind = {Cylinder = {radius = 5, height = 10, angle_deg = 360}}}
                assert(#pc.doc.rebuild() == 0)
                local path = os.tmpname() .. ".stl"
                pc.file.export{path = path, bodies = {model}}
                pc.doc.set_visible{id = model, visible = false}
                pc.file.import{path = path, scan = true}
                pc.doc.rebuild()
                os.remove(path)
                local scan
                for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
                local axis = pc.inspect.grow{kind = "cylinder", scan = scan, at = {5,0,5}, method = "circumscribed"}.element
                local bottom = pc.inspect.grow{kind = "plane", scan = scan, at = {0,0,0}}.element
                local top = pc.inspect.grow{kind = "plane", scan = scan, at = {0,0,10}}.element
                local middle = pc.inspect.construct{made = "plane", from = {bottom,top}}.element
                local comparison = pc.inspect.compare{scan = scan, reference = model}.comparison
                local section = pc.inspect.section{scan = scan}.section
                local position = pc.inspect.tolerance{characteristic = "position", element = axis,
                  datums = {{element = bottom}}, basic = {0,0,0}, limit = 0.05,
                  material = "mmc", material_size = 10.2}
                assert(position.pass and position.value < 0.01 and position.bonus > 0.19)
                for _, kind in ipairs({"surface_profile", "line_profile"}) do
                  local reading = pc.inspect.tolerance{characteristic = kind, element = axis,
                    comparison = comparison, section = section, limit = 0.1, unequal = 0.01}
                  assert(reading.pass and reading.value < 0.1)
                end
                for _, kind in ipairs({"circular_runout", "total_runout"}) do
                  local reading = pc.inspect.tolerance{characteristic = kind, element = axis,
                    datum = axis, limit = 0.05, section_width = 0.1}
                  assert(reading.pass and reading.value < 0.05)
                end
                local symmetry = pc.inspect.tolerance{characteristic = "symmetry", element = bottom,
                  opposed = top, datum = middle, limit = 0.05}
                assert(symmetry.pass and symmetry.value < 0.01)
                assert(#pc.inspect.report{}.tolerances == 6)
            "#),
        tolerance_command,
    );
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
