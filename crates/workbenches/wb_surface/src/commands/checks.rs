//! Reading a body's surfaces: `surface.check` lists how its faces join,
//! `surface.curvature` and `surface.zebra` paint it.

use core_document::{
    CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, ParamKind,
    WorkbenchRuntimeContext,
};
use serde_json::{Value, json};

use crate::{CHECK_TOOL, CURVATURE_TOOL, SurfaceWorkbench, ZEBRA_TOOL, analysis, join_label};

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable<SurfaceWorkbench>) {
    t.add_on(check(), |bench, args, ctx| {
        bench.check_by_command(args, ctx)
    });
    t.add_on_by_id(curvature(), analyse);
    t.add_on_by_id(zebra(), analyse);
    t.explain(prose());
}

/// The notes, related commands and examples of this file's commands.
pub(super) fn prose() -> &'static core_document::CommandProse {
    static PROSE: std::sync::OnceLock<core_document::CommandProse> = std::sync::OnceLock::new();
    core_document::CommandProse::once(&PROSE, "checks.md", include_str!("checks.md"), &[])
}

/// Paint the analysis command `id` names.
fn analyse(
    bench: &mut SurfaceWorkbench,
    id: &str,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    bench.analyse_by_command(id, args, ctx)
}

/// `surface.check`.
pub fn check() -> CommandSpec {
    CommandSpec::new(
        CHECK_TOOL,
        "Measure how a body's faces meet at each shared edge",
    )
    .param("body", ParamKind::Id, "The body, or a feature in it")
    .returns(
        "a list of {point, gap, angle_deg, curvature, join}, one per shared edge; the view \
         labels them",
    )
    .read_only()
}

/// `surface.curvature`.
pub fn curvature() -> CommandSpec {
    CommandSpec::new(
        CURVATURE_TOOL,
        "Paint a body with how sharply its surfaces bend",
    )
    .param("body", ParamKind::Id, "The body, or a feature in it")
    .optional(
        "measure",
        ParamKind::String,
        "gaussian (the default), mean, max or min",
    )
    .optional(
        "limit",
        ParamKind::Number,
        "The curvature the colours reach at either end; 0 (the default) takes it from \
             the body",
    )
    .returns("{measure, low, high, unit}: the measure's range over the body")
    .read_only()
}

/// `surface.zebra`.
pub fn zebra() -> CommandSpec {
    CommandSpec::new(ZEBRA_TOOL, "Paint a body with zebra stripes")
        .param("body", ParamKind::Id, "The body, or a feature in it")
        .optional(
            "stripes",
            ParamKind::Number,
            "Dark stripes per half turn of the surface (6), 1 to 64",
        )
        .optional(
            "axis",
            ParamKind::String,
            "X, Y or Z (the default): the way they run",
        )
        .returns("nothing")
        .read_only()
}

impl SurfaceWorkbench {
    /// `surface.check`: the body's joins, shown in the view as the tool
    /// shows them.
    fn check_by_command(
        &mut self,
        args: &CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> CommandResult {
        let body = core_document::command::Args(args).body_or_feature_body(ctx.document, "body")?;
        self.check_body(ctx, body).map_err(CommandError::failed)?;
        let joins = self
            .tab
            .check
            .as_ref()
            .map(|c| c.joins.as_slice())
            .unwrap_or_default();
        Ok(Value::Array(
            joins
                .iter()
                .map(|(at, j)| {
                    json!({
                        "point": at,
                        "gap": j.gap,
                        "angle_deg": j.angle_deg,
                        "curvature": j.curvature,
                        "join": join_label(j),
                    })
                })
                .collect(),
        ))
    }
}

impl SurfaceWorkbench {
    /// `surface.curvature` and `surface.zebra`: the body painted, and for
    /// the map the measure's range over it.
    fn analyse_by_command(
        &mut self,
        id: &str,
        args: &CommandArgs,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> CommandResult {
        let a = core_document::command::Args(args);
        let body = a.body_or_feature_body(ctx.document, "body")?;
        let display = if id == CURVATURE_TOOL {
            let measure = match a.opt_string("measure")? {
                Some(name) => analysis::Measure::from_name(name)
                    .ok_or_else(|| CommandError::bad("measure", "is gaussian, mean, max or min"))?,
                None => analysis::Measure::Gaussian,
            };
            let limit = a.opt_number("limit")?.unwrap_or(0.0);
            if !(limit.is_finite() && limit >= 0.0) {
                return Err(CommandError::bad("limit", "is a curvature of 0 or more"));
            }
            analysis::Display::Curvature { measure, limit }
        } else {
            let stripes = a
                .opt_number("stripes")?
                .unwrap_or(f64::from(analysis::DEFAULT_STRIPES));
            if !(1.0..=64.0).contains(&stripes) {
                return Err(CommandError::bad("stripes", "is a count from 1 to 64"));
            }
            let axis = match a.opt_string("axis")? {
                Some(name) => analysis::StripeAxis::ALL
                    .into_iter()
                    .find(|x| x.label() == name)
                    .ok_or_else(|| CommandError::bad("axis", "is X, Y or Z"))?,
                None => analysis::StripeAxis::Z,
            };
            analysis::Display::Zebra {
                stripes: stripes.round() as u32,
                axis,
            }
        };
        self.analyse(ctx, body, display)
            .map_err(CommandError::failed)?;
        let Some(analysis) = &self.tab.analysis else {
            return Ok(Value::Null);
        };
        if let analysis::Display::Curvature { measure, .. } = display {
            let (low, high) = analysis
                .range(ctx.document, ctx.kernel, &ctx.sketch_palette)
                .ok_or_else(|| CommandError::failed("the kernel read no curvature of the body"))?;
            return Ok(
                json!({"measure": measure.name(), "low": low, "high": high, "unit": measure.unit()}),
            );
        }
        Ok(Value::Null)
    }
}
