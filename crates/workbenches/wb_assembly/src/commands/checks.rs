//! Checks of the whole assembly: interference and mass.

use core_document::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, ParamKind, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};

use super::{BOX, DEFAULT_DENSITY, body_list};

/// Whether `id` is one of these commands.
pub(super) fn owns(id: &str) -> bool {
    matches!(id, "asm.interference" | "asm.mass")
}

/// These commands' specs.
pub(super) fn configure(specs: &mut Vec<CommandSpec>) {
    specs.push(
        CommandSpec::new(
            "asm.interference",
            "Where solid bodies share material: each pair that clashes, how much \
             and where",
        )
        .optional(
            "bodies",
            ParamKind::List,
            "Only these bodies; every visible one when left out",
        )
        .optional(
            "clearance",
            ParamKind::Number,
            "Look instead for pairs nearer than this many mm",
        )
        .returns(
            "{checked, skipped, clashes, unchecked}, each clash {a, b, volume (mm³), \
             centre}; skipped counts visible bodies with no solid, unchecked lists the \
             pairs the kernel failed on as {a, b, error}. With a clearance, {checked, \
             skipped, near, unchecked}, each near pair {a, b, distance (mm), on_a, on_b}, \
             nearest first",
        )
        .read_only()
        .note(
            "Faces that only touch, a body resting on another, are no clash. `volume` is the \
             shared material in mm³ and `centre` its middle, in world space.",
        )
        .note(
            "`clearance` answers the nearby pairs instead of the clashes, with the nearest \
             point on each body; it takes longer.",
        )
        .see_also("asm.motion_clashes")
        .see_also("asm.mass")
        .example(
            "A lid sunk 1 mm into its base",
            &format!(
                "{BOX}{}",
                r#"
            local base = box(0, 20, 20, 5)
            local lid = box(5, 10, 10, 3)
            assert(#pc.doc.rebuild() == 0)
            pc.asm.move{body = lid, by = {0, 0, 5}}
            assert(#pc.asm.interference{}.clashes == 0, "resting on the top is no clash")
            pc.asm.move{body = lid, by = {0, 0, -1}}
            local found = pc.asm.interference{}
            assert(found.checked == 2 and #found.clashes == 1)
            local clash = found.clashes[1]
            assert(math.abs(clash.volume - 10 * 10 * 1) < 1e-3, "1 mm of the lid sunk in")
            assert(math.abs(clash.centre[3] - 4.5) < 1e-3)
                "#
            ),
        ),
    );
    specs.push(
        CommandSpec::new(
            "asm.mass",
            "The mass, centre of mass and inertia of the solid bodies at one density",
        )
        .optional(
            "bodies",
            ParamKind::List,
            "Only these bodies; every visible one when left out",
        )
        .optional(
            "density",
            ParamKind::Number,
            "g/cm³ for bodies without a material (1 when left out)",
        )
        .optional(
            "bought",
            ParamKind::Bool,
            "Count the bought parts' bodies (true when left out); false weighs the made parts \
             alone",
        )
        .returns(
            "{mass (g), volume (mm³), centre = {x, y, z} or nil, inertia = {tensor, \
             principal} or nil, bodies = {{body, mass, volume, centre, bought}, ...}, skipped}",
        )
        .read_only()
        .note(
            "`centre` is the centre of mass in world space, mm. An id in `bodies` that is not \
             a body is passed over, so a list of feature ids answers a mass of 0.",
        )
        .note(
            "`inertia` is about the centre of mass, g·mm² on the world's axes, each body's \
             moved there by the parallel axis theorem where it sits: `tensor`, three rows, and \
             `principal`, the principal moments smallest first, each {moment, axis}.",
        )
        .note(
            "Bought parts weigh in, as the parts list counts them among the assembly's parts; \
             each body says whether it is `bought`, and `bought = false` leaves them out.",
        )
        .see_also("asm.parts")
        .see_also("asm.interference")
        .example(
            "A block's mass at 1.24 g/cm³",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 20}
            local block = pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
            assert(#pc.doc.rebuild() == 0)
            local m = pc.asm.mass{density = 1.24}
            assert(math.abs(m.volume - 2000) < 1e-3)
            assert(math.abs(m.mass - 2.48) < 1e-6, "2 cm³ at 1.24 g/cm³")
            assert(m.centre[1] == 10 and m.centre[3] == 2.5)
            -- About its centre, g·mm²: m (20² + 20²) / 12 about Z.
            assert(math.abs(m.inertia.tensor[3][3] - 2.48 * 800 / 12) < 1e-6)
            "#,
        ),
    );
}

/// Run command `id`, one of these.
pub(super) fn run(
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let a = Args(args);
    match id {
        "asm.mass" => {
            let kernel = ctx
                .kernel
                .ok_or_else(|| CommandError::failed("no kernel to measure with"))?;
            let among = body_list(&a)?;
            let density = a
                .opt_number("density")?
                .unwrap_or(f64::from(DEFAULT_DENSITY));
            let bought = crate::parts::bought_bodies(ctx.document, &ctx.bought_kinds);
            let report = crate::mass::plan(ctx.document, among.as_deref(), &bought)
                .run(
                    kernel,
                    &std::sync::atomic::AtomicUsize::new(0),
                    &std::sync::atomic::AtomicBool::new(false),
                )
                .map_err(CommandError::failed)?;
            let report = if a.opt_bool("bought")?.unwrap_or(true) {
                report
            } else {
                report.made_only()
            };
            let bodies: Vec<Value> = report
                .bodies
                .iter()
                .map(|b| {
                    json!({
                        "body": b.body.0.to_string(),
                        "mass": b.mass_g(density),
                        "volume": b.volume_mm3,
                        "centre": b.centre,
                        "bought": b.bought,
                    })
                })
                .collect();
            let inertia = report.inertia(density).map(|inertia| {
                json!({
                    "tensor": inertia.about_centre,
                    "principal": inertia
                        .principal
                        .map(|(moment, axis)| json!({"moment": moment, "axis": axis})),
                })
            });
            Ok(json!({
                "mass": report.mass_g(density),
                "volume": report.volume_mm3(),
                "centre": report.centre(density),
                "inertia": inertia,
                "bodies": bodies,
                "skipped": report.skipped,
            }))
        }
        "asm.interference" => {
            let kernel = ctx
                .kernel
                .ok_or_else(|| CommandError::failed("no kernel to check with"))?;
            let among = body_list(&a)?;
            if let Some(gap) = a.opt_number("clearance")? {
                let found =
                    crate::interference::plan_clearance(ctx.document, among.as_deref(), gap)
                        .run(
                            kernel,
                            &std::sync::atomic::AtomicUsize::new(0),
                            &std::sync::atomic::AtomicBool::new(false),
                        )
                        .map_err(CommandError::failed)?;
                let near: Vec<Value> = found
                    .near
                    .iter()
                    .map(|n| {
                        json!({
                            "a": n.a.0.to_string(),
                            "b": n.b.0.to_string(),
                            "distance": n.distance_mm,
                            "on_a": n.on_a,
                            "on_b": n.on_b,
                        })
                    })
                    .collect();
                return Ok(json!({
                    "checked": found.checked,
                    "skipped": found.skipped,
                    "near": near,
                    "unchecked": unchecked_json(&found.unchecked),
                }));
            }
            let found = crate::interference(ctx.document, kernel, among.as_deref())
                .map_err(CommandError::failed)?;
            let clashes: Vec<Value> = found
                .clashes
                .iter()
                .map(|c| {
                    json!({
                        "a": c.a.0.to_string(),
                        "b": c.b.0.to_string(),
                        "volume": c.volume_mm3,
                        "centre": c.centre,
                    })
                })
                .collect();
            Ok(json!({
                "checked": found.checked,
                "skipped": found.skipped,
                "clashes": clashes,
                "unchecked": unchecked_json(&found.unchecked),
            }))
        }
        _ => Err(CommandError::Unknown(id.to_string())),
    }
}

/// The pairs an interference check could not compare, as `{a, b, error}`.
fn unchecked_json(unchecked: &[crate::interference::Unchecked]) -> Vec<Value> {
    unchecked
        .iter()
        .map(|u| json!({"a": u.a.0.to_string(), "b": u.b.0.to_string(), "error": u.why}))
        .collect()
}
