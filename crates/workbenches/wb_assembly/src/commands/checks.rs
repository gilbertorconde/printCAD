//! Checks of the whole assembly: interference and mass.

use core_document::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, ParamKind,
    WorkbenchRuntimeContext,
};
use serde_json::{Value, json};

use super::{BOX, DEFAULT_DENSITY, body_list};

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable) {
    t.add(
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
        .read_only(),
        interference,
    );
    t.add(
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
        .read_only(),
        mass,
    );
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands, the
/// examples' setup as `{{box}}`.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(
        &PROSE,
        "checks.md",
        include_str!("checks.md"),
        &[("box", BOX)],
    )
}

fn mass(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
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

fn interference(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("no kernel to check with"))?;
    let among = body_list(&a)?;
    if let Some(gap) = a.opt_number("clearance")? {
        let found = crate::interference::plan_clearance(ctx.document, among.as_deref(), gap)
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

/// The pairs an interference check could not compare, as `{a, b, error}`.
fn unchecked_json(unchecked: &[crate::interference::Unchecked]) -> Vec<Value> {
    unchecked
        .iter()
        .map(|u| json!({"a": u.a.0.to_string(), "b": u.b.0.to_string(), "error": u.why}))
        .collect()
}
