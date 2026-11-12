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
    .note(
        "One entry per edge two faces of the body share: `gap` in mm between them, \
         `angle_deg` the crease (0 where they meet tangent), `point` halfway along the edge. \
         Any body with a shape is checked, a Design solid too.",
    )
    .note(
        "`curvature` is the largest difference, in 1/mm, between how sharply the two faces \
         bend square to the edge (nil where it could not be read); `join` is how the view \
         labels it: G2 where they meet tangent and bend alike, G1 tangent, else the crease \
         angle, or the gap where they are apart.",
    )
    .note(
        "Sheets that only touch share no edge until sewn, so the seam between them is not \
         listed: two separate sheets give an empty list. A body not built yet is refused \
         (\"The body has no shape to check yet\").",
    )
    .see_also("surface.sew")
    .see_also("surface.fillet")
    .example(
        "A sharp corner, then rounded",
        r#"
        local s = pc.sketch.new{plane = "XY"}
        pc.sketch.polyline{sketch = s, points = {{10, 0}, {0, 0}, {0, 10}}}
        local walls = pc.surface.extrude{sketches = {s}, length = 5}
        local body = pc.doc.feature{id = walls}.body
        assert(#pc.doc.rebuild() == 0)
        local joins = pc.surface.check{body = body}
        assert(#joins == 1 and math.abs(joins[1].angle_deg - 90) < 1e-6, "one square crease")
        assert(joins[1].gap < 1e-9)
        pc.surface.fillet{body = body, radius = 2,
          edges = {{point = joins[1].point, direction = {0, 0, 1}}}}
        assert(#pc.doc.rebuild() == 0)
        joins = pc.surface.check{body = body}
        assert(#joins == 2, "the round meets each wall")
        for _, j in ipairs(joins) do
          assert(j.angle_deg < 0.01 and j.join == "G1", "tangent, the curvature jumps")
          assert(math.abs(j.curvature - 0.5) < 1e-3, "from flat to a 2 mm round")
        end
        "#,
    )
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
    .note(
        "`mean`, `max` and `min` are signed against the faces' outward normals: negative \
             where a surface bulges out, positive in a hollow. `gaussian` is the product of the two \
             principal curvatures (1/mm²): positive on a dome or in a bowl, negative on a \
             saddle, zero on a plane and on what unrolls flat (a cylinder, a cone). `mean` \
             is their average, `max` and `min` each (1/mm).",
    )
    .note(
        "The view paints the body until the task closes, or until `surface.zebra` or \
             `surface.check` takes its place. A body not built yet is refused (\"The body has \
             no shape to paint yet\").",
    )
    .see_also("surface.zebra")
    .see_also("surface.check")
    .example(
        "A tube's side bends one way only",
        r#"
            local s = pc.sketch.new{plane = "XZ"}
            pc.sketch.line{sketch = s, x1 = 5, y1 = 0, x2 = 5, y2 = 10}
            local tube = pc.surface.revolve{sketches = {s}}
            assert(#pc.doc.rebuild() == 0)
            local body = pc.doc.feature{id = tube}.body
            local k = pc.surface.curvature{body = body}
            assert(math.abs(k.low) < 1e-9 and math.abs(k.high) < 1e-9, "unrolls flat")
            k = pc.surface.curvature{body = body, measure = "mean"}
            assert(math.abs(math.abs(k.low) - 0.1) < 1e-6 and math.abs(k.high - k.low) < 1e-6,
              "half of 1/5 everywhere")
            "#,
    )
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
        .note(
            "Each stripe is a band of the way the surface faces, turned about `axis`. Across a \
             crease the stripes break, across a tangent join they bend sharply, and across a \
             curvature continuous one they run on smoothly. The view paints the body until \
             the task closes.",
        )
        .see_also("surface.curvature")
        .see_also("surface.check")
        .example(
            "Stripes on a tube",
            r#"
            local s = pc.sketch.new{plane = "XZ"}
            pc.sketch.line{sketch = s, x1 = 5, y1 = 0, x2 = 5, y2 = 10}
            local body = pc.doc.feature{id = pc.surface.revolve{sketches = {s}}}.body
            assert(#pc.doc.rebuild() == 0)
            assert(pc.surface.zebra{body = body, stripes = 8} == nil)
            local ok = pcall(pc.surface.zebra, {body = body, axis = "W"})
            assert(not ok, "X, Y or Z")
            "#,
        )
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
