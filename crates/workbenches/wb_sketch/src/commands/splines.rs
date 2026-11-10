//! B-splines: converting curves, degree, knots, weights and joining curves
//! into one.

use core_document::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, FeatureId, ParamKind,
    WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::sketch::Sketch;

use super::{ids, ids_of, load, made_since, points, sketch};

/// Whether `id` is one of these commands.
pub(super) fn owns(id: &str) -> bool {
    matches!(
        id,
        "sketch.to_bspline"
            | "sketch.spline_degree"
            | "sketch.insert_knot"
            | "sketch.knot_multiplicity"
            | "sketch.spline_knots"
            | "sketch.spline_weight"
            | "sketch.join"
    )
}

/// These commands' specs.
pub(super) fn configure(specs: &mut Vec<CommandSpec>) {
    specs.push(
        sketch(CommandSpec::new(
            "sketch.to_bspline",
            "Make lines, arcs, circles, ellipses and conics into splines that are exactly them",
        ))
        .param("items", ParamKind::List, "The curves to make splines of")
        .returns("{elements}: what it made")
        .note(
            "Each curve is replaced and its id is gone; an arc's ends stay, as the spline's \
             first and last control points. Arcs and circles become rational splines, \
             exact.",
        )
        .note(
            "A spline is kind \"other\" in `pc.sketch.geometry`; its control points, degree, \
             knots and weights are in `pc.doc.feature{id = s}.fields.sketch.geometry`.",
        )
        .note("It returns {elements, constraints}: the new control points and the spline.")
        .see_also("sketch.join")
        .see_also("sketch.spline_degree")
        .example(
            "A circle made a spline pads the same disc",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local c = pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 5}
            local made = pc.sketch.to_bspline{sketch = s, items = {c}}
            assert(#made.elements > 0)
            for _, e in ipairs(pc.sketch.geometry{sketch = s}) do
              assert(e.id ~= c, "the circle is replaced")
            end
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - math.pi * 25) < 0.01, "exactly the circle: " .. m.volume)
            "#,
        ),
    );
    specs.push(
        sketch(CommandSpec::new(
            "sketch.spline_degree",
            "Raise or lower the degree of splines: raising keeps the curve, lowering fits the \
             nearest one",
        ))
        .param("items", ParamKind::List, "The splines")
        .param("by", ParamKind::Integer, "1 to raise, -1 to lower")
        .note(
            "Only the sign of `by` counts: any number above 0 raises one degree, any below \
             lowers one, and 0 is refused.",
        )
        .note(
            "Raising adds a control point. The bspline tool draws degree 3, which the data \
             leaves out: `degree` shows in it once changed.",
        )
        .see_also("sketch.spline_knots")
        .see_also("sketch.to_bspline")
        .example(
            "A cubic raised to degree 4",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local made = pc.sketch.draw{sketch = s, tool = "bspline", points = {{0, 0}, {10, 10}, {20, 0}, {30, 10}, "finish"}}
            local spline = made.elements[#made.elements]
            local function shape()
              for _, g in ipairs(pc.doc.feature{id = s}.fields.sketch.geometry) do
                if g.BSpline then return g.BSpline end
              end
            end
            assert(#shape().control_points == 4)
            pc.sketch.spline_degree{sketch = s, items = {spline}, by = 1}
            assert(shape().degree == 4 and #shape().control_points == 5)
            "#,
        ),
    );
    specs.push(
        sketch(CommandSpec::new(
            "sketch.insert_knot",
            "Insert a knot into a spline where it passes nearest a point, the curve unchanged",
        ))
        .param("spline", ParamKind::Id, "The spline")
        .param("at", ParamKind::List, "A point near the curve, {x, y}")
        .note(
            "`at` need not lie on the curve: the knot goes at the parameter where the spline \
             passes nearest it.",
        )
        .note("Only a spline takes it; a line, arc or circle is refused (\"is not a spline\").")
        .see_also("sketch.knot_multiplicity")
        .see_also("sketch.spline_knots")
        .example(
            "A knot inserted halfway",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local made = pc.sketch.draw{sketch = s, tool = "bspline", points = {{0, 0}, {10, 10}, {20, 0}, {30, 10}, "finish"}}
            local spline = made.elements[#made.elements]
            assert(#pc.sketch.spline_knots{sketch = s, spline = spline} == 0)
            pc.sketch.insert_knot{sketch = s, spline = spline, at = {15, 5}}
            local knots = pc.sketch.spline_knots{sketch = s, spline = spline}
            assert(#knots == 1 and knots[1].multiplicity == 1)
            assert(math.abs(knots[1].knot - 0.5) < 1e-6, "halfway along this symmetric spline")
            "#,
        ),
    );
    specs.push(
        sketch(CommandSpec::new(
            "sketch.knot_multiplicity",
            "Set how many times a spline's knot stands (1 up to the degree), or remove it with 0",
        ))
        .param("spline", ParamKind::Id, "The spline")
        .param(
            "knot",
            ParamKind::Number,
            "The knot's value, as sketch.spline_knots lists it",
        )
        .param("multiplicity", ParamKind::Integer, "")
        .note(
            "`knot` is a value `sketch.spline_knots` lists; one where the spline has no knot \
             is refused.",
        )
        .note(
            "A multiplicity above the degree is held at the degree. Asking for the one the \
             knot has is refused (\"the knot is unchanged\").",
        )
        .see_also("sketch.spline_knots")
        .see_also("sketch.insert_knot")
        .example(
            "A knot doubled, then removed",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local made = pc.sketch.draw{sketch = s, tool = "bspline", points = {{0, 0}, {10, 10}, {20, 0}, {30, 10}, "finish"}}
            local spline = made.elements[#made.elements]
            pc.sketch.insert_knot{sketch = s, spline = spline, at = {15, 5}}
            local knot = pc.sketch.spline_knots{sketch = s, spline = spline}[1].knot
            pc.sketch.knot_multiplicity{sketch = s, spline = spline, knot = knot, multiplicity = 2}
            assert(pc.sketch.spline_knots{sketch = s, spline = spline}[1].multiplicity == 2)
            pc.sketch.knot_multiplicity{sketch = s, spline = spline, knot = knot, multiplicity = 0}
            assert(#pc.sketch.spline_knots{sketch = s, spline = spline} == 0, "0 removes it")
            "#,
        ),
    );
    specs.push(
        sketch(CommandSpec::new(
            "sketch.spline_knots",
            "A spline's knots inside its ends and how many times each stands",
        ))
        .param("spline", ParamKind::Id, "The spline")
        .returns("{{knot, multiplicity}}")
        .read_only()
        .note(
            "The knots at the ends are not listed: a spline fresh from the bspline tool \
             lists none.",
        )
        .note("An element that is not a spline answers an empty list rather than an error.")
        .see_also("sketch.insert_knot")
        .see_also("sketch.knot_multiplicity")
        .example(
            "A half circle as a spline has one double knot",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local arc = pc.sketch.arc{sketch = s, x = 0, y = 0, radius = 5, start = 0, ["end"] = 180}
            local made = pc.sketch.to_bspline{sketch = s, items = {arc}}
            local spline = made.elements[#made.elements]
            local knots = pc.sketch.spline_knots{sketch = s, spline = spline}
            assert(#knots == 1, "the knots at its ends are not listed")
            assert(knots[1].knot == 0.5 and knots[1].multiplicity == 2)
            "#,
        ),
    );
    specs.push(
        sketch(CommandSpec::new(
            "sketch.spline_weight",
            "Weigh a spline's control point: more pulls the curve toward it",
        ))
        .param("spline", ParamKind::Id, "The spline")
        .param("point", ParamKind::Id, "One of its control points")
        .param("weight", ParamKind::Number, "More than 0; 1 is plain")
        .note(
            "`point` is one of the spline's `control_points` in `pc.doc.feature`'s data, \
             where its `weights` stand in the same order; any other point is refused.",
        )
        .note("A weight of 0 or less is refused.")
        .see_also("sketch.to_bspline")
        .example(
            "A control point weighed three times",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local made = pc.sketch.draw{sketch = s, tool = "bspline", points = {{0, 0}, {10, 10}, {20, 0}, {30, 10}, "finish"}}
            local spline = made.elements[#made.elements]
            local function shape()
              for _, g in ipairs(pc.doc.feature{id = s}.fields.sketch.geometry) do
                if g.BSpline then return g.BSpline end
              end
            end
            local second = shape().control_points[2]
            pc.sketch.spline_weight{sketch = s, spline = spline, point = second, weight = 3}
            local weights = shape().weights
            assert(weights[2] == 3 and weights[1] == 1)
            "#,
        ),
    );
    specs.push(
        sketch(CommandSpec::new(
            "sketch.join",
            "Merge curves that meet end to end into one B-spline following them",
        ))
        .param(
            "items",
            ParamKind::List,
            "The lines, arcs, arcs of ellipses, parabolas and hyperbolas, and open splines to merge",
        )
        .optional(
            "tolerance",
            ParamKind::Number,
            "How far the spline may stray from the curves, mm (0.01)",
        )
        .returns("{elements}: what it made")
        .note(
            "The curves are replaced by one spline ending where the chain ends; their ids \
             are gone.",
        )
        .note(
            "It needs two or more curves meeting end to end in one chain: one curve, a gap \
             or a branch is refused. A sharp corner, such as a rectangle's, fails at the \
             default `tolerance` (\"No spline follows these curves within 0.01 mm\").",
        )
        .see_also("sketch.to_bspline")
        .example(
            "A line and a quarter arc made one spline",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local line = pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
            local arc = pc.sketch.arc{sketch = s, x = 10, y = 5, radius = 5, start = -90, ["end"] = 0}
            local made = pc.sketch.join{sketch = s, items = {line, arc}}
            assert(#made.elements > 0)
            local count = {}
            for _, e in ipairs(pc.sketch.geometry{sketch = s}) do count[e.kind] = (count[e.kind] or 0) + 1 end
            assert(count.line == nil and count.arc == nil, "both are replaced")
            assert(count.other == 1, "by one spline")
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
    let sketch_id = FeatureId(a.id("sketch")?);
    let mut feature = load(ctx, sketch_id)?;
    let sketch = &mut feature.sketch;
    let answer = match id {
        "sketch.to_bspline" => {
            let items: std::collections::HashSet<Uuid> = ids(args.get("items"), "items", sketch)?
                .into_iter()
                .collect();
            let before = ids_of(sketch);
            let effect = crate::spline_edit::to_bspline(sketch, &items);
            if !effect.changed {
                return Err(CommandError::failed(
                    effect
                        .log
                        .unwrap_or_else(|| "nothing to convert".to_string()),
                ));
            }
            made_since(sketch, &before)
        }
        "sketch.spline_degree" => {
            let items: std::collections::HashSet<Uuid> = ids(args.get("items"), "items", sketch)?
                .into_iter()
                .collect();
            let by = a.number("by")? as i32;
            if by == 0 {
                return Err(CommandError::bad("by", "must be 1 or -1"));
            }
            let effect = crate::spline_edit::change_degree(sketch, &items, by.signum());
            if !effect.changed {
                return Err(CommandError::failed(
                    effect
                        .log
                        .unwrap_or_else(|| "no degree changed".to_string()),
                ));
            }
            Value::Null
        }
        "sketch.insert_knot" => {
            let spline = one_id(args, "spline", sketch)?;
            let at = points(Some(&json!([args
                .get("at")
                .cloned()
                .unwrap_or(Value::Null)])))
            .map_err(|_| CommandError::bad("at", "must be {x, y}"))?[0];
            let t = crate::spline_edit::param_near(sketch, spline, at)
                .ok_or_else(|| CommandError::bad("spline", "is not a spline"))?;
            let effect = crate::spline_edit::add_knot(sketch, spline, t);
            if !effect.changed {
                return Err(CommandError::failed(
                    effect.log.unwrap_or_else(|| "no knot inserted".to_string()),
                ));
            }
            Value::Null
        }
        "sketch.knot_multiplicity" => {
            let spline = one_id(args, "spline", sketch)?;
            let knot = a.number("knot")?;
            let multiplicity = a.number("multiplicity")?;
            if multiplicity < 0.0 {
                return Err(CommandError::bad("multiplicity", "must be 0 or more"));
            }
            let effect = if multiplicity < 1.0 {
                crate::spline_edit::remove_knot(sketch, spline, knot)
            } else {
                crate::spline_edit::set_multiplicity(sketch, spline, knot, multiplicity as usize)
            };
            if !effect.changed {
                return Err(CommandError::failed(
                    effect
                        .log
                        .unwrap_or_else(|| "the knot is unchanged".to_string()),
                ));
            }
            Value::Null
        }
        "sketch.spline_knots" => {
            let spline = one_id(args, "spline", sketch)?;
            let knots = crate::spline_edit::knots_of(sketch, spline);
            return Ok(json!(
                knots
                    .iter()
                    .map(|(k, m)| json!({"knot": k, "multiplicity": m}))
                    .collect::<Vec<_>>()
            ));
        }
        "sketch.spline_weight" => {
            let spline = one_id(args, "spline", sketch)?;
            let point = one_id(args, "point", sketch)?;
            let effect = crate::spline_edit::set_weight(sketch, spline, point, a.number("weight")?);
            if !effect.changed {
                return Err(CommandError::failed(
                    effect
                        .log
                        .unwrap_or_else(|| "the weight is unchanged".to_string()),
                ));
            }
            Value::Null
        }
        "sketch.join" => {
            let items: std::collections::HashSet<Uuid> = ids(args.get("items"), "items", sketch)?
                .into_iter()
                .collect();
            let tolerance = a
                .opt_number("tolerance")?
                .unwrap_or(crate::tools::JOIN_TOLERANCE);
            if tolerance <= 0.0 {
                return Err(CommandError::bad("tolerance", "must be more than 0"));
            }
            let before = ids_of(sketch);
            let effect = crate::tools::join(sketch, &items, tolerance);
            if !effect.changed {
                return Err(CommandError::failed(
                    effect.log.unwrap_or_else(|| "nothing to join".to_string()),
                ));
            }
            made_since(sketch, &before)
        }
        _ => return Err(CommandError::Unknown(id.to_string())),
    };
    crate::solver::solve(sketch);
    ctx.document
        .update_feature_data(sketch_id, feature.to_json())
        .map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document.mark_feature_dirty(sketch_id);
    Ok(answer)
}

/// The one id `args` gives as `name`.
fn one_id(
    args: &serde_json::Map<String, Value>,
    name: &str,
    sketch: &Sketch,
) -> Result<Uuid, CommandError> {
    let value = args
        .get(name)
        .cloned()
        .ok_or_else(|| CommandError::bad(name, "is missing"))?;
    Ok(ids(Some(&json!([value])), name, sketch)?[0])
}
