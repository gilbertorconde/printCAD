//! The sketcher's commands: make a sketch and draw in it by numbers, for
//! scripts and other callers that are not a click.
//!
//! Coordinates are the sketch's own, in millimetres. A drawn end that lands
//! exactly on a point the sketch already has takes that point, as a
//! snapped click does, so lines drawn end to end close a profile.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, DatumFeature, DatumShape,
    FeatureId, FileImport, ParamKind, WorkbenchContext, WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::feature::DatumSupport;
use crate::feature::SketchFeature;
use crate::sketch::{
    Arc, Circle, ConstraintKind, GeometryElement, Line, Point, Sketch, SketchPlane, Vec2D,
};

/// Register every command this module runs.
pub fn register(context: &mut WorkbenchContext) {
    let sketch = |spec: CommandSpec| spec.param("sketch", ParamKind::Id, "The sketch to draw in");
    context.register_command(
        placing(CommandSpec::new(
            "sketch.new",
            "Make an empty sketch on a base plane",
        ))
        .optional(
            "generator",
            ParamKind::String,
            "gear, sprocket, shaft or keyway: the sketch is that generator's, at its default \
             numbers, centred on the plane's origin",
        )
        .returns("the sketch's id")
        .note(
            "Without `body` the sketch goes in the selected body, else in a new one: in a \
             script two sketches made without `body` land in two bodies, and a pocket or a \
             hole from the second is refused for want of material. Give `body` from \
             `pc.doc.feature{id = ...}.body` or `pc.doc.new_body`.",
        )
        .note(
            "`body` takes any body, a surface body too: that is how a sketch starts in a \
             surface body, as the Surface bench's Create sketch does.",
        )
        .note(
            "XY faces +Z, YZ faces +X and XZ faces -Y, so a pad from an XZ sketch grows \
             toward -Y and `offset` moves an XZ sketch toward -Y. The sketch's x and y run \
             along the plane's two letters (on XZ, y is world Z). Lower case names are \
             taken too.",
        )
        .note(
            "With `generator` it is named after it (Sprocket, Sprocket_1, ...) and \
             `sketch.generator` sets its numbers, as `design.sprocket` and its kin do on a \
             base plane or a picked face.",
        )
        .see_also("doc.new_body")
        .see_also("sketch.rect")
        .see_also("design.datum")
        .example(
            "Two sketches in one body",
            r#"
            local body = pc.doc.new_body{name = "Bracket"}
            local base = pc.sketch.new{body = body, plane = "XY"}
            local side = pc.sketch.new{body = body, plane = "XZ", offset = 5}
            assert(pc.doc.feature{id = base}.body == body)
            assert(pc.doc.feature{id = side}.body == body)
            assert(#pc.doc.bodies() == 1, "both sketches went in the one body")
            "#,
        ),
    );
    context.register_command(
        placing(CommandSpec::new(
            "sketch.import_dxf",
            "Make a sketch of a DXF drawing: its lines, arcs, circles, ellipses and polylines \
             as sketch curves, splines as lines through points on them, hidden ones as \
             construction, ends that meet sharing one point",
        ))
        .param("path", ParamKind::String, "The DXF file")
        .optional(
            "scale",
            ParamKind::Number,
            "Millimetres per drawing unit; the drawing's own unit when left out, else 1",
        )
        .returns("the sketch's id")
        .note(
            "Left out, `scale` follows the drawing's own unit: a drawing in inches comes in \
             at 25.4 mm a unit. A drawing that names no unit comes in at 1 mm a unit.",
        )
        .note(
            "It is placed as `sketch.new` places a sketch: without `body` it goes in the \
             selected body, else a new one. It is named after the file unless `name` says \
             otherwise.",
        )
        .note(
            "A file that cannot be read or parsed is refused, and so is a drawing with no \
             curves.",
        )
        .see_also("sketch.new")
        .see_also("sketch.repair"),
    );
    context.register_import(FileImport::new("DXF drawing", ["dxf"], "sketch.import_dxf"));
    let placed = |spec: CommandSpec| {
        spec.optional("x", ParamKind::Number, "Its middle, mm; 0 when left out")
            .optional("y", ParamKind::Number, "Its middle, mm; 0 when left out")
            .optional("width", ParamKind::Number, "How wide it lies, mm")
            .optional(
                "angle",
                ParamKind::Number,
                "Its turn counter-clockwise, degrees",
            )
            .optional("opacity", ParamKind::Number, "How much of it shows, 0 to 1")
    };
    context.register_command(
        placed(placing(CommandSpec::new(
            "sketch.image",
            "Lay a picture (PNG or JPEG) on a sketch's plane to draw over: in the sketch \
             given or being edited, else in a new one",
        )))
        .param("path", ParamKind::String, "The picture file")
        .optional("sketch", ParamKind::Id, "The sketch it goes in")
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
    );
    context.register_import(FileImport::new(
        "Reference image (sketch)",
        ["png", "jpg", "jpeg"],
        "sketch.image",
    ));
    context.register_command(
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
    );
    context.register_command(
        sketch(CommandSpec::new("sketch.point", "Add a point"))
            .param("x", ParamKind::Number, "")
            .param("y", ParamKind::Number, "")
            .returns("the point's id")
            .note(
                "A point the sketch already has exactly at (x, y) is returned rather than a \
                 second one made.",
            )
            .note(
                "Lines, arcs and circles drawn later with an end or a centre exactly on it \
                 take it: making the points first is how a script knows the ids of the \
                 ends it constrains.",
            )
            .see_also("sketch.constrain")
            .see_also("sketch.geometry")
            .example(
                "A corner made first and shared",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                local corner = pc.sketch.point{sketch = s, x = 10, y = 5}
                pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 5}
                local ends = 0
                for _, g in ipairs(pc.doc.feature{id = s}.fields.sketch.geometry) do
                  if g.Line and (g.Line.start == corner or g.Line["end"] == corner) then ends = ends + 1 end
                end
                assert(ends == 2, "the rectangle's corner is the point made first")
                "#,
            ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.line",
            "Add a line from (x1, y1) to (x2, y2)",
        ))
        .param("x1", ParamKind::Number, "")
        .param("y1", ParamKind::Number, "")
        .param("x2", ParamKind::Number, "")
        .param("y2", ParamKind::Number, "")
        .returns("the line's id")
        .note(
            "Each end takes a point the sketch has exactly there, else a new one: lines \
             drawn end to end share their ends and close a profile.",
        )
        .note(
            "Unlike `sketch.polyline` and `sketch.rect`, it adds no constraint: a level line \
             is not held level.",
        )
        .note(
            "Its ends are points of their own: the line's `start` and `end` in \
             `pc.doc.feature{id = s}.fields.sketch.geometry`, or points made first with \
             `sketch.point`. Two ends at the same spot are refused (\"a line needs two \
             different ends\").",
        )
        .see_also("sketch.polyline")
        .see_also("sketch.point")
        .example(
            "Three lines end to end close a triangle",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 30, y2 = 0}
            pc.sketch.line{sketch = s, x1 = 30, y1 = 0, x2 = 0, y2 = 20}
            pc.sketch.line{sketch = s, x1 = 0, y1 = 20, x2 = 0, y2 = 0}
            assert(#pc.sketch.constraints{sketch = s} == 0, "nothing holds a line level")
            local pad = pc.design.pad{sketch = s, length = 4}
            assert(#pc.doc.rebuild() == 0, "the ends are shared, so the triangle closes")
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - 30 * 20 / 2 * 4) < 1e-3)
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.polyline",
            "Add lines through a list of points, each ending where the next starts; a level \
             or upright one is held so",
        ))
        .param("points", ParamKind::List, "Points as {x, y} pairs")
        .optional(
            "closed",
            ParamKind::Bool,
            "Join the last point to the first",
        )
        .returns("the lines' ids")
        .note(
            "A point is `{x, y}` or `{x = .., y = ..}`. Ending on the first point closes the \
             outline as `closed = true` does: an end landing exactly on a point the sketch \
             has takes that point.",
        )
        .note(
            "Only closed loops count in a profile: an open polyline is left out of what a \
             pad or pocket uses, and a sketch with no closed loop fails at `pc.doc.rebuild()` \
             with \"profile is not closed\", not when the feature is made.",
        )
        .see_also("sketch.rect")
        .see_also("sketch.line")
        .example(
            "A closed triangle padded",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local lines = pc.sketch.polyline{sketch = s, points = {{0, 0}, {30, 0}, {0, 20}}, closed = true}
            assert(#lines == 3)
            local pad = pc.design.pad{sketch = s, length = 5}
            assert(#pc.doc.rebuild() == 0, "the triangle closes")
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - 30 * 20 / 2 * 5) < 1e-3)
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.rect",
            "Add a rectangle from its corner (x, y), its width and its height, its sides held \
             level and upright",
        ))
        .param("x", ParamKind::Number, "")
        .param("y", ParamKind::Number, "")
        .param("width", ParamKind::Number, "")
        .param("height", ParamKind::Number, "")
        .returns("the four lines' ids")
        .note(
            "(x, y) is a corner, not the centre: a rectangle centred on the origin starts \
             at (-width / 2, -height / 2). A negative width or height draws it to the left \
             or below.",
        )
        .note(
            "Its sides are held level and upright but carry no dimensions; \
             `pc.sketch.constrain` adds them.",
        )
        .see_also("sketch.polyline")
        .see_also("sketch.constrain")
        .example(
            "A plate centred on the origin",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local sides = pc.sketch.rect{sketch = s, x = -20, y = -15, width = 40, height = 30}
            assert(#sides == 4)
            local pad = pc.design.pad{sketch = s, length = 3}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - 40 * 30 * 3) < 1e-3)
            assert(math.abs(m.centre[1]) < 1e-6 and math.abs(m.centre[2]) < 1e-6, "centred")
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new("sketch.circle", "Add a circle"))
            .param("x", ParamKind::Number, "The centre")
            .param("y", ParamKind::Number, "The centre")
            .param("radius", ParamKind::Number, "")
            .returns("the circle's id")
            .note("It takes the radius, not the diameter; a radius of 0 or less is refused.")
            .note(
                "A circle inside a closed outline of the same sketch is a hole in what is \
                 padded from it; circles apart from each other pad as separate solids in \
                 one body.",
            )
            .note(
                "`pc.design.hole` reads only a circle's centre: the hole's size is its own \
                 `diameter`, whatever the circle's radius.",
            )
            .see_also("design.hole")
            .example(
                "A washer: a ring padded from two circles",
                r#"
                local s = pc.sketch.new{plane = "XY"}
                pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 10}
                pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 4}
                local pad = pc.design.pad{sketch = s, length = 2}
                assert(#pc.doc.rebuild() == 0)
                local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
                assert(math.abs(m.volume - math.pi * (100 - 16) * 2) < 0.01, m.volume)
                "#,
            ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.arc",
            "Add an arc, counter-clockwise from the start angle to the end angle",
        ))
        .param("x", ParamKind::Number, "The centre")
        .param("y", ParamKind::Number, "The centre")
        .param("radius", ParamKind::Number, "")
        .param(
            "start",
            ParamKind::Number,
            "Degrees from the sketch's X axis",
        )
        .param("end", ParamKind::Number, "Degrees from the sketch's X axis")
        .returns("the arc's id")
        .note(
            "Angles are degrees, and the arc runs counter-clockwise from `start` to `end`: \
             -90 to 90 is the right half, 90 to -90 the left.",
        )
        .note("It takes the radius, not the diameter; a radius of 0 or less is refused.")
        .note(
            "Its centre and both ends are points of their own, each taking a point the \
             sketch has exactly there, so a line drawn to an end joins it.",
        )
        .see_also("sketch.circle")
        .see_also("sketch.draw")
        .example(
            "A half disc closed by a line",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.arc{sketch = s, x = 0, y = 0, radius = 10, start = -90, ["end"] = 90}
            pc.sketch.line{sketch = s, x1 = 0, y1 = 10, x2 = 0, y2 = -10}
            local pad = pc.design.pad{sketch = s, length = 2}
            assert(#pc.doc.rebuild() == 0, "the line ends on the arc's ends")
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - math.pi * 100 / 2 * 2) < 0.01, m.volume)
            assert(m.min[1] > -1e-6, "counter-clockwise from -90 to 90 is the right half")
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.geometry",
            "List the sketch's elements with their points",
        ))
        .returns("a list of {id, kind, points, radius?, construction}")
        .read_only()
        .note(
            "A line's points are its start and end, an arc's its centre, start and end, a \
             circle's and an ellipse's their centre; positions are as last solved, with the \
             values formulas give its dimensions (what builds, which \
             `pc.doc.feature{id = s}.fields` may not yet be). Each element also says whether \
             it is `external`.",
        )
        .note(
            "Ends and centres are listed again as elements of kind point. A spline, a \
             parabola or a hyperbola is kind \"other\" with no points: its control points \
             are in `pc.doc.feature{id = s}.fields.sketch.geometry`.",
        )
        .see_also("sketch.constraints")
        .see_also("doc.feature")
        .example(
            "Ends and centres are points of their own",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local l = pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
            pc.sketch.circle{sketch = s, x = 5, y = 5, radius = 2}
            local count = {}
            for _, e in ipairs(pc.sketch.geometry{sketch = s}) do
              count[e.kind] = (count[e.kind] or 0) + 1
              if e.id == l then assert(e.points[2][1] == 10 and e.points[2][2] == 0) end
            end
            assert(count.line == 1 and count.circle == 1)
            assert(count.point == 3, "the line's ends and the centre are points of their own")
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.constrain",
            "Constrain elements, as the constraint's toolbar button does for a selection",
        ))
        .param(
            "kind",
            ParamKind::String,
            "coincident, point_on_object, midpoint, horizontal, vertical, \
             horizontal_vertical, parallel, perpendicular, tangent, equal, symmetric, block, group, lock, dimension, distance, \
             distance_x, distance_y, gap, arc_length, radius, diameter, radius_diameter, \
             angle, angle_x, angle_y, angle_at_point, arc_angle, angle_three_points (items: \
             arm, corner, arm), ellipse_minor or refraction; radius on an ellipse is its \
             major radius, arc_length on a spline or conic its length",
        )
        .param(
            "items",
            ParamKind::List,
            "Element ids, or \"origin\", \"x_axis\" and \"y_axis\"",
        )
        .optional(
            "value",
            ParamKind::Number,
            "A dimension's value (mm, degrees for an angle, the ratio of indices for a \
             refraction); the measured one when left out",
        )
        .optional(
            "remove_redundant",
            ParamKind::Bool,
            "Take away the older constraints the new ones make redundant",
        )
        .returns("the new constraints' ids")
        .note(
            "Points are elements of their own: a line's ends are the point ids at its \
             `start` and `end` in `pc.doc.feature{id = s}.fields.sketch.geometry`, the line's \
             own id is the line. A point made with `sketch.point` before the line is the same \
             id as the end drawn on it.",
        )
        .note(
            "Without `value` a dimension takes what it measures on the sketch as it stands. \
             Angles are degrees, a diameter the diameter, a radius the radius.",
        )
        .note(
            "\"distance\" on one line is its length (listed as Length), on two points the \
             distance between them, on a point and a curve or two curves the gap. \
             \"dimension\" picks as the toolbar does: a line's length, a circle's diameter, \
             an arc's radius, two lines' angle (their distance when parallel).",
        )
        .note(
            "Nothing stays put until constrained: a dimension on free geometry moves every \
             item it names, so tie a corner to \"origin\" first to keep it where it was \
             drawn.",
        )
        .note(
            "\"lock\" holds a point by its distances along X and Y from the origin, each \
             the size of a coordinate (a point at x = -5 takes 5 and stays at -5).",
        )
        .note(
            "\"group\" makes its items one rigid unit, keeping the shape they have now: it \
             moves and turns as a whole, three degrees of freedom, and a drag of any member \
             carries all of it. Constraints among the members alone that say nothing of the \
             axes rest while the group holds; of those that do (horizontal, vertical, an angle \
             to an axis), the first turns the whole group and the rest rest.",
        )
        .note(
            "A constraint that contradicts others is still added; `pc.sketch.status` names \
             the conflict. While it stands the sketch does not solve, gives no profile, and \
             what is built from it fails at `pc.doc.rebuild()` (\"the sketch does not solve: \
             its constraints conflict\"). `remove_redundant` takes away only older \
             constraints the new one repeats, never one it contradicts.",
        )
        .note(
            "A kind that does not fit the items is refused (\"the ... constraint does not \
             fit these items\"), one that is no kind at all is refused with the list of \
             kinds, and so is a `value` for a kind that takes none.",
        )
        .see_also("sketch.status")
        .see_also("sketch.set_value")
        .see_also("sketch.constraints")
        .example(
            "A plate fully constrained from its corner on the origin",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local corner = pc.sketch.point{sketch = s, x = 1, y = 1}
            local sides = pc.sketch.rect{sketch = s, x = 1, y = 1, width = 20, height = 10}
            pc.sketch.constrain{sketch = s, kind = "coincident", items = {corner, "origin"}}
            pc.sketch.constrain{sketch = s, kind = "distance", items = {sides[1]}, value = 40}
            pc.sketch.constrain{sketch = s, kind = "distance", items = {sides[2]}, value = 25}
            assert(pc.sketch.status{sketch = s}.dof == 0, "fully constrained")
            local pad = pc.design.pad{sketch = s, length = 2}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - 40 * 25 * 2) < 1e-3)
            assert(math.abs(m.min[1]) < 1e-6 and math.abs(m.min[2]) < 1e-6, "its corner on the origin")
            "#,
        )
        .example(
            "A rectangle grouped and dragged as one",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local sides = pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            pc.sketch.constrain{sketch = s, kind = "group", items = sides}
            local status = pc.sketch.status{sketch = s}
            assert(status.dof == 2, "square to the axes, free to slide")
            assert(#status.redundant == 0, "its sides' alignments rest in the group")
            pc.sketch.drag{sketch = s, items = {sides[1]}, by = {5, 3}}
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - 20 * 10) < 1e-2, "the same rectangle")
            assert(math.abs(m.min[1] - 5) < 1e-4 and math.abs(m.min[2] - 3) < 1e-4, "moved whole")
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.set_value",
            "Change a dimension's value",
        ))
        .param("constraint", ParamKind::Id, "")
        .param("value", ParamKind::Number, "mm, or degrees for an angle")
        .optional(
            "driving",
            ParamKind::Bool,
            "false makes it a reference dimension that only measures",
        )
        .note(
            "`constraint` is an id `sketch.constrain` returned; an element's id, or a \
             constraint that is not a dimension, is refused.",
        )
        .note("Angles are degrees; a diameter takes the diameter, a radius the radius.")
        .note(
            "To bind a dimension to a formula, `doc.set_formula` takes it by the key \
             `doc.parameters` lists for the sketch, which is the constraint's id.",
        )
        .see_also("sketch.constrain")
        .see_also("doc.parameters")
        .see_also("doc.set_formula")
        .example(
            "A circle's diameter changed after it was dimensioned",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local c = pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 5}
            local d = pc.sketch.constrain{sketch = s, kind = "diameter", items = {c}}
            pc.sketch.set_value{sketch = s, constraint = d[1], value = 20}
            assert(pc.sketch.constraints{sketch = s}[1].value == 20)
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - math.pi * 10 ^ 2) < 0.01, m.volume)
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.draw",
            "Run a drawing or editing tool over points of the sketch, as clicks there would",
        ))
        .param(
            "tool",
            ParamKind::String,
            "line, polyline, rect, rect_center, rect_rounded, rect3, rect_center3, rect_frame, \
             circle, circle3, arc, arc3, \
             ellipse, ellipse3, ellipse_arc, parabola, hyperbola, bspline, polygon, slot, arc_slot, point, fillet, \
             chamfer, trim, extend, split, bspline_knot, offset, translate, rotate, scale or mirror",
        )
        .param(
            "points",
            ParamKind::List,
            "The clicks, each {x, y}, or {x = , y = , typed = {length = 20}, constrain = true} \
             with values typed at it, or for the line tool {x = , y = , arc = true}: an arc \
             there, tangent to what ends where it draws from; \"arc\" and \"line\" switch a \
             polyline, \"finish\" ends a spline",
        )
        .optional(
            "tolerance",
            ParamKind::Number,
            "How close a click snaps onto points and curves, mm (0.001)",
        )
        .optional(
            "params",
            ParamKind::Any,
            "Tool settings: polygon_sides, slot_width, fillet_radius, chamfer_length, corner_keep, \
             offset_distance, offset_round, offset_both, offset_delete, offset_linked, copies, \
             copies_linked, copies_total, copies_symmetric, rotate_full_turn, \
             bspline_periodic, bspline_degree, bspline_interpolate, auto_constraints, \
             mirror_keep, mirror_linked, \
             mirror_center, cut_at_axes",
        )
        .optional(
            "construction",
            ParamKind::Bool,
            "What it makes is construction geometry",
        )
        .optional(
            "avoid_redundant",
            ParamKind::Bool,
            "Drop auto constraints that add nothing (true)",
        )
        .optional(
            "selection",
            ParamKind::List,
            "The elements offset, translate, rotate, scale and mirror act on",
        )
        .returns("{elements, constraints}: what it made")
        .note(
            "Each click is in the sketch's millimetres and snaps as a click in the view \
             does: one on the origin is held there and one on an axis is held on it, the \
             constraints coming with what is made.",
        )
        .note(
            "A tool takes the clicks its shape needs: line, circle (centre, then a point on \
             it) and slot (the ends of its centre line) two; rect_center its centre, then a \
             corner; ellipse its centre, an end of the major axis, then a point on it. Clicks \
             short of a shape make nothing, without an error.",
        )
        .note(
            "Sizes not clicked come from `params`: `slot_width` 4 mm, `polygon_sides` 6, \
             `fillet_radius` and `chamfer_length` 2 mm when not given. A fillet or chamfer \
             takes one click on the corner. A bspline ends with the word \"finish\" in \
             `points`.",
        )
        .note(
            "A value typed at a click (`typed = {length = 20}`) sets the shape's size; with \
             `constrain = true` it is kept as a dimension. The names are length, angle, \
             width, height, diameter, radius, major_radius, minor_radius, sweep (an arc's \
             turn, degrees counter-clockwise) and x and y (a shape's first point).",
        )
        .note(
            "It returns every element made, end points and centres included, and every \
             constraint made with them.",
        )
        .see_also("sketch.polyline")
        .see_also("sketch.rect")
        .see_also("sketch.circle")
        .example(
            "A slot drawn by its centre line",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local made = pc.sketch.draw{sketch = s, tool = "slot", points = {{0, 0}, {20, 0}}, params = {slot_width = 6}}
            assert(#made.elements > 0 and #made.constraints > 0)
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - (20 * 6 + math.pi * 3 ^ 2)) < 1e-3, m.volume)
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.drag",
            "Drag elements by a step, the rest of the sketch following its constraints",
        ))
        .param("items", ParamKind::List, "The elements to drag")
        .param("by", ParamKind::List, "The step, {x, y}")
        .note(
            "`by` is a step {dx, dy} in mm, not a place to go to: the items move by it as \
             far as their constraints let them, and the rest of the sketch follows.",
        )
        .note(
            "A point held by \"lock\" stays where it is. A corner of a rectangle from \
             `sketch.rect` stretches it, the opposite corner staying put.",
        )
        .note("Dragging a text block's point moves the whole text.")
        .see_also("sketch.set_value")
        .example(
            "A rectangle stretched by its corner",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local corner = pc.sketch.point{sketch = s, x = 20, y = 10}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            pc.sketch.drag{sketch = s, items = {corner}, by = {10, 5}}
            for _, e in ipairs(pc.sketch.geometry{sketch = s}) do
              if e.id == corner then
                assert(math.abs(e.points[1][1] - 30) < 1e-4 and math.abs(e.points[1][2] - 15) < 1e-4)
              end
            end
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - 30 * 15) < 1e-2, "still a rectangle, stretched")
            "#,
        ),
    );
    context.register_command(
        sketch(
            CommandSpec::new(
                "sketch.attachment",
                "Move a sketch on the datum it is attached to: along its normal, across it, \
                 turned about it",
            )
            .optional("offset", ParamKind::Number, "Along the normal, mm")
            .optional("shift", ParamKind::List, "Across the plane, {x, y} in mm")
            .optional("turn", ParamKind::Number, "About the normal, degrees"),
        )
        .note(
            "Only a sketch made with `on` (a datum plane or coordinate system) takes it; \
             one on a base plane, a plane of its own or an `attachment` is refused (\"is not \
             attached to a datum\").",
        )
        .note(
            "Each value given replaces the one the sketch had rather than adding to it: \
             `offset = 5` twice leaves it 5 mm off the datum.",
        )
        .note("`shift` runs along the datum's own x and y; `turn` is degrees about its normal.")
        .see_also("sketch.new")
        .see_also("design.datum")
        .see_also("sketch.set_plane")
        .example(
            "A sketch set 5 mm off its datum",
            r#"
            local body = pc.doc.new_body{name = "Plate"}
            local datum = pc.design.datum{body = body, kind = "plane", plane = "XY", offset = {0, 0, 10}}
            local s = pc.sketch.new{body = body, on = datum}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 4, height = 2}
            pc.sketch.attachment{sketch = s, offset = 5}
            pc.sketch.attachment{sketch = s, offset = 5}
            pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = body}
            assert(math.abs(m.min[3] - 15) < 1e-6, "5 mm off the datum at 10, however often it is set")
            "#,
        ),
    );
    context.register_command(
        sketch(
            CommandSpec::new(
                "sketch.external_from",
                "Bring another sketch's curves and points, or a datum, into this sketch as \
                 external geometry that follows them",
            )
            .param("from", ParamKind::Id, "A sketch or a datum")
            .optional(
                "counts",
                ParamKind::Bool,
                "true: it counts in the profile, as drawn geometry does; false (the default): \
                 it only guides the sketch",
            )
            .returns("the external elements made"),
        )
        .note(
            "Every curve and loose point of the other sketch comes, its construction left \
             out; a datum comes as one element.",
        )
        .note(
            "It only guides unless `counts = true`: a sketch holding guides alone has no \
             profile, and a pad of it fails at `pc.doc.rebuild()`.",
        )
        .note(
            "It returns {elements, constraints}: the curves and the points at their ends. \
             Only the curves are external geometry, which `pc.sketch.geometry` marks \
             `external`.",
        )
        .note("`from` must be a sketch other than this one, or a datum; anything else is refused.")
        .note(
            "The elements follow their source without this sketch being opened: when the \
             other sketch or the datum moves (by hand or by a formula), `pc.sketch.geometry` \
             reads them where it now is and the next `pc.doc.rebuild()` builds from that. A \
             source curve that becomes another kind of curve, or is deleted, is caught up \
             with when this sketch is next edited.",
        )
        .note(
            "What the sketch already holds comes once: bringing the same sketch again adds \
             only what is new in it, and is refused (\"... already in the sketch ...\") when \
             nothing is.",
        )
        .see_also("sketch.external_defining")
        .see_also("sketch.carbon_copy")
        .example(
            "A circle from the sketch below, counted, padded and following its source",
            r#"
            local body = pc.doc.new_body{name = "Boss"}
            local base = pc.sketch.new{body = body, plane = "XY"}
            local circle = pc.sketch.circle{sketch = base, x = 0, y = 0, radius = 5}
            local radius = pc.sketch.constrain{sketch = base, kind = "radius", items = {circle}, value = 5}
            local top = pc.sketch.new{body = body, plane = "XY", offset = 10}
            local made = pc.sketch.external_from{sketch = top, from = base, counts = true}
            assert(#made.elements == 2, "the circle and its centre")
            pc.design.pad{sketch = top, length = 3}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = body}
            assert(math.abs(m.volume - math.pi * 25 * 3) < 0.01, m.volume)
            assert(math.abs(m.min[3] - 10) < 1e-6)
            pc.sketch.set_value{sketch = base, constraint = radius[1], value = 6}
            assert(#pc.doc.rebuild() == 0)
            m = pc.doc.measure{body = body}
            assert(math.abs(m.volume - math.pi * 36 * 3) < 0.01, "the copy follows: " .. m.volume)
            "#,
        ),
    );
    context.register_command(
        sketch(
            CommandSpec::new(
                "sketch.external_defining",
                "Count external geometry in the sketch's profiles, or leave it only guiding",
            )
            .param("items", ParamKind::List, "External elements' ids")
            .optional(
                "on",
                ParamKind::Bool,
                "true counts them (the default), false stops",
            ),
        )
        .note(
            "Every item must be external geometry, or the call is refused (\"... is not \
             external geometry\"); the elements `pc.sketch.geometry` marks `external` are. \
             The points at an external curve's ends and centre go with their curve, so the \
             `elements` `sketch.external`, `sketch.external_from` and `sketch.intersection` \
             return can be passed as they come.",
        )
        .note(
            "Counting takes an element out of construction; `on = false` makes it a guide \
             again.",
        )
        .see_also("sketch.external_from")
        .see_also("sketch.external")
        .example(
            "A guide made to count",
            r#"
            local body = pc.doc.new_body{name = "Boss"}
            local base = pc.sketch.new{body = body, plane = "XY"}
            pc.sketch.circle{sketch = base, x = 0, y = 0, radius = 5}
            local top = pc.sketch.new{body = body, plane = "XY", offset = 10}
            local made = pc.sketch.external_from{sketch = top, from = base}
            local pad = pc.design.pad{sketch = top, length = 3}
            assert(#pc.doc.rebuild() == 1, "a guide alone is no profile")
            pc.sketch.external_defining{sketch = top, items = made.elements}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = body}
            assert(math.abs(m.volume - math.pi * 25 * 3) < 0.01, m.volume)
            "#,
        ),
    );
    context.register_command(
        sketch(
            CommandSpec::new(
                "sketch.solver_settings",
                "How far the solver goes on this sketch",
            )
            .optional(
                "iterations",
                ParamKind::Number,
                "The most steps it takes (100 when never set)",
            )
            .optional(
                "tolerance",
                ParamKind::Number,
                "How small what is left must be, against the sketch's size (1e-9 when never \
                 set)",
            ),
        )
        .note(
            "`iterations` must be at least 1 and `tolerance` above 0 and below 1; either may \
             be left out to keep what the sketch has.",
        )
        .note(
            "The sketch keeps them (`fields.sketch.solver` in `pc.doc.feature`) and is \
             solved again with them at once.",
        )
        .see_also("sketch.status")
        .example(
            "Settings kept on the sketch",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 5}
            pc.sketch.solver_settings{sketch = s, iterations = 500, tolerance = 1e-6}
            local solver = pc.doc.feature{id = s}.fields.sketch.solver
            assert(solver.max_iterations == 500 and solver.tolerance == 1e-6)
            assert(pc.sketch.status{sketch = s}.solved)
            "#,
        ),
    );
    context.register_command(
        sketch(
            CommandSpec::new(
                "sketch.repair",
                "Join ends of curves that nearly meet, and remove curves of no size, doubled \
                 curves and constraints left naming nothing",
            )
            .optional(
                "tolerance",
                ParamKind::Number,
                "How near two ends must be to join, mm (0.01 when left out)",
            )
            .returns("what was repaired, in words"),
        )
        .note(
            "It is the answer to a profile that fails with \"profile is not closed\" because \
             ends miss by a hair, as a drawing brought in may.",
        )
        .note(
            "It answers in words, such as \"2 end(s) joined, 1 duplicate curve(s) removed\", \
             or \"nothing to repair\".",
        )
        .see_also("sketch.import_dxf")
        .see_also("sketch.status")
        .example(
            "Ends that miss by microns joined",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
            pc.sketch.line{sketch = s, x1 = 10.005, y1 = 0, x2 = 0, y2 = 10}
            pc.sketch.line{sketch = s, x1 = 0, y1 = 10, x2 = 0, y2 = 0.003}
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 1, "two ends miss by a few microns")
            local said = pc.sketch.repair{sketch = s}
            assert(said:find("2 end"), said)
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - 50) < 0.01, m.volume)
            assert(pc.sketch.repair{sketch = s} == "nothing to repair")
            "#,
        ),
    );
    context.register_command(
        sketch(
            CommandSpec::new(
                "sketch.restore",
                "Put the sketch back as `data` holds it: an editing session cancelled",
            )
            .param(
                "data",
                ParamKind::Any,
                "The sketch as doc.feature lists its data",
            ),
        )
        .note(
            "`data` is the whole `fields` of `pc.doc.feature{id = s}`, plane included, taken \
             before the edits; a table that is not a sketch's data is refused.",
        )
        .see_also("doc.feature")
        .example(
            "Edits put back",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 5}
            local saved = pc.doc.feature{id = s}.fields
            pc.sketch.circle{sketch = s, x = 5, y = 2, radius = 1}
            assert(#pc.sketch.geometry{sketch = s} == 10)
            pc.sketch.restore{sketch = s, data = saved}
            assert(#pc.sketch.geometry{sketch = s} == 8, "the circle and its centre are gone")
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.set_plane",
            "Move the sketch onto another plane, its geometry kept in its own coordinates",
        ))
        .param("normal", ParamKind::List, "The plane's normal, {x, y, z}")
        .optional("origin", ParamKind::List, "Its origin, {x, y, z}")
        .optional(
            "x_axis",
            ParamKind::List,
            "The sketch's X direction, {x, y, z}",
        )
        .note(
            "The geometry keeps its sketch coordinates and moves with the plane. Without \
             `x_axis` the sketch's x is a direction square to the normal chosen for it (+Y \
             for a normal along X), so give `x_axis` to know which way the geometry lies.",
        )
        .note(
            "The plane given is fixed: a sketch made on a datum stops following it, and \
             `sketch.attachment` refuses it from then on.",
        )
        .see_also("sketch.attachment")
        .see_also("sketch.new")
        .example(
            "A sketch moved onto a plane facing +X",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 4}
            pc.sketch.set_plane{sketch = s, normal = {1, 0, 0}, origin = {5, 0, 0}, x_axis = {0, 1, 0}}
            local pad = pc.design.pad{sketch = s, length = 2}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.min[1] - 5) < 1e-6 and math.abs(m.max[1] - 7) < 1e-6, "padded along +X from x = 5")
            assert(math.abs(m.max[2] - 10) < 1e-6 and math.abs(m.max[3] - 4) < 1e-6, "sketch x along Y, y along Z")
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.array",
            "Repeat elements in rows and columns",
        ))
        .param("items", ParamKind::List, "The elements to repeat")
        .param("rows", ParamKind::Integer, "")
        .param("cols", ParamKind::Integer, "")
        .param("dx", ParamKind::Number, "The step between columns, mm")
        .param("dy", ParamKind::Number, "The step between rows, mm")
        .optional(
            "linked",
            ParamKind::Bool,
            "Copies stay the originals' size, spaced by one pitch along the rows and one \
             down the columns (false)",
        )
        .returns("{elements}: what it made")
        .note(
            "The items are the first copy: `rows = 2, cols = 3` makes five more. Columns \
             step along the sketch's x by `dx`, rows along its y by `dy`.",
        )
        .note(
            "`rows` and `cols` must be at least 1, and one of them more than 1 (\"an array \
             needs elements and at least two rows or columns\").",
        )
        .note(
            "Without `linked` the copies are free geometry; with it, constraints keep them \
             the originals' size and on the pitch.",
        )
        .note(
            "It returns {elements, constraints}: the copies, their points and centres \
             included.",
        )
        .see_also("sketch.draw")
        .see_also("design.linear_pattern")
        .example(
            "A plate with six holes in two rows",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 50, height = 30}
            local hole = pc.sketch.circle{sketch = s, x = 10, y = 10, radius = 2}
            local made = pc.sketch.array{sketch = s, items = {hole}, rows = 2, cols = 3, dx = 15, dy = 10}
            assert(#made.elements == 10, "five more circles and their centres: the original is the first")
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - (50 * 30 - 6 * math.pi * 4)) < 0.01, m.volume)
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.text",
            "Lay out text as closed outlines standing on a new point: the start of its first \
             line on the baseline",
        ))
        .param(
            "text",
            ParamKind::String,
            "What it says; a new line starts a line",
        )
        .param("at", ParamKind::List, "Where its point goes, {x, y}")
        .optional(
            "font",
            ParamKind::String,
            "IBM Plex Sans, IBM Plex Sans SemiBold, IBM Plex Mono, the family of a font \
             installed on the machine, or a font file's path (IBM Plex Sans)",
        )
        .optional(
            "font_asset",
            ParamKind::String,
            "The document asset a font file was kept as, which a recording names",
        )
        .note(
            "A font installed on the machine, named by its family, or a font file named by \
             its path, is kept in the document the first time it is used, so the text is \
             made again from it on any machine; a font neither carried nor kept is refused \
             with why. A browser page has no installed fonts.",
        )
        .optional("size", ParamKind::Number, "The font's em, mm (10)")
        .optional(
            "spacing",
            ParamKind::Number,
            "Added between letters, mm (0)",
        )
        .optional(
            "angle",
            ParamKind::Number,
            "Degrees it turns about its point (0)",
        )
        .returns("{text, point}: the block and the point it stands on")
        .note(
            "`at` is where the baseline starts: the letters stand on it, capitals reaching \
             about 0.7 of `size`, the em.",
        )
        .note(
            "The letters are closed outlines that pad as they read; dragging the point \
             returned moves the whole text.",
        )
        .note(
            "A `font` that is not one of the three bundled names is a file's path when it \
             looks like one (a path separator, or a font file's extension) or names a file \
             there, else an installed family, its letters' case aside; the family a font \
             gives is what the block records. Text with nothing to draw is refused.",
        )
        .see_also("sketch.text_edit")
        .example(
            "Letters padded from the baseline",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local t = pc.sketch.text{sketch = s, text = "PC", at = {0, 0}, size = 10}
            assert(t.text and t.point)
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.min[2]) < 0.2, "the baseline is at y = 0")
            assert(m.max[2] > 6 and m.max[2] < 10, "capitals stand under the 10 mm em")
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.text_edit",
            "Change a text block, its outlines made again where its point stands",
        ))
        .param("block", ParamKind::Id, "The text block, or its point")
        .optional("text", ParamKind::String, "")
        .optional("font", ParamKind::String, "")
        .optional("font_asset", ParamKind::String, "")
        .optional("size", ParamKind::Number, "mm")
        .optional("spacing", ParamKind::Number, "mm")
        .optional("angle", ParamKind::Number, "degrees")
        .note("`block` is either the `text` or the `point` that `sketch.text` returned.")
        .note(
            "Only what is given changes, and the outlines are made again on the same point; \
             a `size` of 0 or less is refused.",
        )
        .note(
            "A block's size, spacing and angle are numbers formulas can set: \
             `pc.doc.parameters` of the sketch lists them under the keys `<block>/size`, \
             `<block>/spacing` and `<block>/angle`, and the outlines are made again at what \
             they come to, in the font the document keeps. The string is not, since a \
             formula makes a number.",
        )
        .see_also("sketch.text")
        .see_also("doc.set_formula")
        .example(
            "Text whose height a variable sets",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local t = pc.sketch.text{sketch = s, text = "I", at = {0, 0}, size = 10}
            pc.var.new{name = "Label"}
            pc.var.set{set = "Label", name = "height", formula = "10 mm"}
            pc.doc.set_formula{id = s, parameter = t.text .. "/size", formula = "Label.height"}
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local body = pc.doc.feature{id = pad}.body
            local small = pc.doc.measure{body = body}
            pc.var.set{set = "Label", name = "height", formula = "20 mm"}
            assert(#pc.doc.rebuild() == 0)
            local large = pc.doc.measure{body = body}
            assert(math.abs(large.max[2] - 2 * small.max[2]) < 1e-3, "twice as tall")
            "#,
        )
        .example(
            "A letter made twice as large",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local t = pc.sketch.text{sketch = s, text = "I", at = {0, 0}, size = 10}
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local body = pc.doc.feature{id = pad}.body
            local small = pc.doc.measure{body = body}
            pc.sketch.text_edit{sketch = s, block = t.text, size = 20}
            assert(#pc.doc.rebuild() == 0)
            local large = pc.doc.measure{body = body}
            assert(math.abs(large.max[2] - 2 * small.max[2]) < 1e-3, "twice as tall")
            assert(math.abs(large.volume - 4 * small.volume) < 1e-3, "four times the area")
            "#,
        ),
    );
    context.register_command(
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
    context.register_command(
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
    context.register_command(
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
    context.register_command(
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
    context.register_command(
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
    context.register_command(
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
    context.register_command(
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
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.set_constraint",
            "Make constraints driving or reference, active or not, parked or not",
        ))
        .param("items", ParamKind::List, "The constraints")
        .optional(
            "driving",
            ParamKind::Bool,
            "false: a reference dimension that only measures",
        )
        .optional("active", ParamKind::Bool, "false: kept but not solved")
        .optional(
            "as",
            ParamKind::String,
            "\"radius\" or \"diameter\": a circle's or arc's dimension measures that instead",
        )
        .optional(
            "parked",
            ParamKind::Bool,
            "true: its symbol moves to the parked layer, drawn only while that layer shows; \
             it still solves",
        )
        .note(
            "`items` are constraint ids; an element's id among them is refused. Only the \
             flags given change, and `driving = false` is refused for a constraint that is \
             not a dimension.",
        )
        .note(
            "A dimension that is not driving measures and conflicts with nothing; a \
             constraint that is not active is kept but left out of solving.",
        )
        .note(
            "`as` turns a radius into a diameter or back, keeping the constraint and the size \
             it holds: its value doubles or halves, as does what a formula on it gives.",
        )
        .see_also("sketch.set_value")
        .see_also("sketch.status")
        .example(
            "A repeated dimension made a reference",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local sides = pc.sketch.rect{sketch = s, x = 0, y = 0, width = 30, height = 15}
            pc.sketch.constrain{sketch = s, kind = "distance", items = {sides[1]}, value = 30}
            local top = pc.sketch.constrain{sketch = s, kind = "distance", items = {sides[3]}}
            assert(#pc.sketch.status{sketch = s}.redundant > 0, "the top's length says the bottom's again")
            pc.sketch.set_constraint{sketch = s, items = top, driving = false}
            assert(#pc.sketch.status{sketch = s}.redundant == 0, "a reference only measures")
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.mirror_sketch",
            "A new sketch on the same plane: this one's geometry mirrored across its Y axis",
        ))
        .returns("the new sketch's id")
        .note(
            "It makes a new sketch, named after this one with \"mirror\", in the same body; \
             this one is left as it is. The mirror is across the sketch's own Y axis: x \
             becomes -x.",
        )
        .note(
            "For both halves in one profile, `sketch.merge` the two, or mirror within one \
             sketch with `sketch.draw`'s mirror tool.",
        )
        .see_also("sketch.merge")
        .see_also("sketch.draw")
        .example(
            "A rectangle mirrored across the Y axis",
            r#"
            local body = pc.doc.new_body{name = "Wing"}
            local s = pc.sketch.new{body = body, plane = "XY"}
            pc.sketch.rect{sketch = s, x = 5, y = 0, width = 10, height = 5}
            local mirrored = pc.sketch.mirror_sketch{sketch = s}
            assert(pc.doc.feature{id = mirrored}.body == body)
            assert(#pc.sketch.geometry{sketch = s} == 8, "the original keeps only its own")
            pc.design.pad{sketch = mirrored, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = body}
            assert(math.abs(m.min[1] + 15) < 1e-6 and math.abs(m.max[1] + 5) < 1e-6, "across the sketch's Y axis")
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.merge",
            "A new sketch holding this one's geometry and other sketches', mapped onto its plane",
        ))
        .param("with", ParamKind::List, "The other sketches")
        .returns("the new sketch's id")
        .note(
            "It makes a new sketch, named after this one with \"merged\", on this one's plane \
             and in its body; the sketches merged are left as they are. Constraints come \
             with the geometry; under a turn or a mirror those stated against the sketch \
             axes follow it (swapped on a quarter turn, made parallel or perpendicular to \
             one another at any other angle). Projected geometry stays projected from its \
             solid, sketch or datum.",
        )
        .note(
            "Each sketch in `with` must lie on a plane parallel to this one (\"its plane is \
             not parallel to this one\"); its geometry is laid onto this plane, the distance \
             between them dropped.",
        )
        .see_also("sketch.carbon_copy")
        .see_also("sketch.mirror_sketch")
        .example(
            "Two sketches merged and padded as one",
            r#"
            local body = pc.doc.new_body{name = "Pair"}
            local left = pc.sketch.new{body = body, plane = "XY"}
            pc.sketch.rect{sketch = left, x = -15, y = 0, width = 10, height = 5}
            local right = pc.sketch.new{body = body, plane = "XY", offset = 3}
            pc.sketch.circle{sketch = right, x = 10, y = 2, radius = 2}
            local merged = pc.sketch.merge{sketch = left, with = {right}}
            assert(#pc.sketch.geometry{sketch = merged} == 10)
            pc.design.pad{sketch = merged, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = body}
            assert(math.abs(m.volume - (50 + math.pi * 4)) < 0.01, m.volume)
            assert(math.abs(m.max[3] - 1) < 1e-6, "on the first sketch's plane")
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.carbon_copy",
            "Copy another sketch's geometry into this one, mapped onto its plane",
        ))
        .param("from", ParamKind::Id, "The sketch to copy")
        .returns("{elements, constraints}: what it made")
        .note(
            "The geometry comes with its constraints, as geometry of this sketch: free to \
             edit, not tied to the sketch it came from. Under a turn or a mirror the \
             constraints stated against the sketch axes follow it, and projected geometry \
             stays projected from its solid, sketch or datum.",
        )
        .note(
            "`from` must lie on a plane parallel to this one (\"its plane is not parallel to \
             this one\"); a sketch cannot copy itself.",
        )
        .see_also("sketch.merge")
        .see_also("sketch.external_from")
        .example(
            "A rectangle copied onto a sketch above it",
            r#"
            local body = pc.doc.new_body{name = "Stack"}
            local base = pc.sketch.new{body = body, plane = "XY"}
            pc.sketch.rect{sketch = base, x = 0, y = 0, width = 10, height = 5}
            local top = pc.sketch.new{body = body, plane = "XY", offset = 5}
            local made = pc.sketch.carbon_copy{sketch = top, from = base}
            assert(#made.elements == 8 and #made.constraints == 4, "the lines, their ends and their constraints")
            pc.design.pad{sketch = top, length = 2}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = body}
            assert(math.abs(m.volume - 100) < 1e-3 and math.abs(m.min[3] - 5) < 1e-6)
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.paste",
            "Add geometry held as a sketch of its own, moved by a step",
        ))
        .param(
            "clipboard",
            ParamKind::Any,
            "The geometry, as a sketch's fields (what copying in the sketcher holds)",
        )
        .param("by", ParamKind::List, "The step, {x, y}")
        .returns("{elements}: what it made")
        .note(
            "`clipboard` is a sketch's own data: `pc.doc.feature{id = s}.fields.sketch` of \
             any sketch serves, and all of its geometry comes, with its constraints.",
        )
        .note("`by` is a step {dx, dy} in mm. It returns {elements, constraints}.")
        .see_also("sketch.carbon_copy")
        .example(
            "A rectangle pasted beside itself",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 5}
            local clip = pc.doc.feature{id = s}.fields.sketch
            local made = pc.sketch.paste{sketch = s, clipboard = clip, by = {20, 0}}
            assert(#made.elements == 8)
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - 100) < 1e-3 and math.abs(m.max[1] - 30) < 1e-6)
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.external",
            "Project edges of solids into the sketch as fixed references",
        ))
        .param(
            "edges",
            ParamKind::List,
            "Each {body, point, direction}: a point on the edge and its direction, \
             in the body's own frame",
        )
        .optional(
            "counts",
            ParamKind::Bool,
            "true: it counts in the profile, as drawn geometry does; false (the default): it \
             only guides the sketch",
        )
        .returns("{elements}: what it made")
        .note(
            "Each edge is a point on it and its direction there, in the body's own frame; \
             `pc.doc.edges` gives both (where the body sits, the same until it is moved). \
             The body must be built first (`pc.doc.rebuild()`), else it is refused (\"that \
             body has no solid shape\").",
        )
        .note(
            "It only guides unless `counts = true`; counted, the projected edges close a \
             profile as drawn lines do. An edge square to the sketch plane projects to a \
             point.",
        )
        .note(
            "It returns {elements, constraints}: the curves and the points at their ends, \
             the curves marked `external` by `pc.sketch.geometry`.",
        )
        .note(
            "The projection follows its solid: rebuilt, the edge is found again by the faces \
             it runs between (else by the point) and projected anew, the sketch unopened. \
             An edge of the sketch's own body is taken from the solid as it stands where the \
             sketch is in the history, so what the sketch builds never moves it; another \
             body's solid built from the sketch is projected again only when the sketch is \
             opened.",
        )
        .note(
            "A point names an edge only within a tenth of the body's diagonal of it; one \
             farther from every edge is refused (\"no edge near ...\"). An edge the sketch \
             already holds comes once: picked again, at any point along it, it is passed \
             over, and a call that brings nothing new is refused (\"... already in the \
             sketch ...\").",
        )
        .see_also("sketch.intersection")
        .see_also("doc.edges")
        .see_also("sketch.external_defining")
        .example(
            "A block's top edges projected and padded higher",
            r#"
            local body = pc.doc.new_body{name = "Block"}
            local base = pc.sketch.new{body = body, plane = "XY"}
            pc.sketch.rect{sketch = base, x = 0, y = 0, width = 20, height = 10}
            pc.design.pad{sketch = base, length = 5}
            assert(#pc.doc.rebuild() == 0)
            local top = pc.sketch.new{body = body, plane = "XY", offset = 5}
            local made = pc.sketch.external{sketch = top, counts = true, edges = {
              {body = body, point = {10, 0, 5}, direction = {1, 0, 0}},
              {body = body, point = {20, 5, 5}, direction = {0, 1, 0}},
              {body = body, point = {10, 10, 5}, direction = {1, 0, 0}},
              {body = body, point = {0, 5, 5}, direction = {0, 1, 0}},
            }}
            assert(#made.elements > 0)
            pc.design.pad{sketch = top, length = 3}
            assert(#pc.doc.rebuild() == 0, "the four top edges close a profile")
            assert(math.abs(pc.doc.measure{body = body}.volume - 20 * 10 * 8) < 1e-3)
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.intersection",
            "Add where faces of solids cross the sketch plane, as fixed references",
        ))
        .param(
            "faces",
            ParamKind::List,
            "Each {body, point, normal}: a point on the face and its normal there, \
             in the body's own frame",
        )
        .optional(
            "counts",
            ParamKind::Bool,
            "true: it counts in the profile, as drawn geometry does; false (the default): it \
             only guides the sketch",
        )
        .returns("{elements}: what it made")
        .note(
            "Each face is a point on it and its outward normal there, in the body's own \
             frame; `pc.doc.faces` gives both. The body must be built first.",
        )
        .note(
            "A face the sketch plane does not cross adds nothing. What it adds only guides \
             unless `counts = true`.",
        )
        .note("It returns {elements, constraints}: the curves and the points at their ends.")
        .see_also("sketch.external")
        .see_also("doc.faces")
        .example(
            "Where a block's top face crosses a sketch through its middle",
            r#"
            local body = pc.doc.new_body{name = "Block"}
            local base = pc.sketch.new{body = body, plane = "XY"}
            pc.sketch.rect{sketch = base, x = 0, y = 0, width = 20, height = 10}
            pc.design.pad{sketch = base, length = 5}
            assert(#pc.doc.rebuild() == 0)
            local cut = pc.sketch.new{body = body, plane = "XZ", offset = -5}
            local made = pc.sketch.intersection{sketch = cut, faces = {{body = body, point = {10, 5, 5}, normal = {0, 0, 1}}}}
            assert(#made.elements == 3, "a line and its two ends")
            for _, e in ipairs(pc.sketch.geometry{sketch = cut}) do
              if e.kind == "line" then
                assert(e.external and e.points[1][2] == 5 and e.points[2][2] == 5, "the top face, at sketch y = 5")
              end
            end
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.constraints",
            "List the sketch's constraints",
        ))
        .returns("a list of {id, kind, items, value?}")
        .read_only()
        .note(
            "`kind` is the stored name (Coincident, Horizontal, Length, Diameter, Angle, \
             ...): \"distance\" on a line lists as Length. `value` is a dimension's, angles \
             in degrees, as its formula sets it when it has one.",
        )
        .note(
            "`items` are the element ids it ties. The origin and the axes show as fixed ids \
             ending in 0001 (origin), 0002 (x axis) and 0003 (y axis), not by name.",
        )
        .see_also("sketch.status")
        .see_also("sketch.set_constraint")
        .example(
            "An angle listed in degrees",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local a = pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
            local b = pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 10, y2 = 10}
            local made = pc.sketch.constrain{sketch = s, kind = "angle", items = {a, b}}
            local listed = pc.sketch.constraints{sketch = s}
            assert(#listed == 1 and listed[1].id == made[1])
            assert(listed[1].kind == "Angle" and math.abs(listed[1].value - 45) < 1e-4, "degrees")
            assert(listed[1].items[1] == a and listed[1].items[2] == b)
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.regions",
            "List the regions the sketch's curves close",
        ))
        .returns("a list of {name, area, x, y, default}")
        .read_only()
        .note(
            "Curves split where they cross, touch or branch, and every closed area between \
             them is a region: two overlapping circles make three, the lens and two \
             crescents. Curves with a loose end are left out.",
        )
        .note(
            "`name` stays while the sketch's dimensions change; a pad, pocket, revolution \
             or groove takes regions by it in its `regions` field. `x`, `y` is a point \
             inside the region, `area` in mm² with its holes taken out. `default` is true \
             for the regions a feature takes when it names none: a region inside another \
             loop is a hole, one inside that is material again.",
        )
        .see_also("design.pad")
        .see_also("sketch.wall_thickness")
        .example(
            "Two crossing circles close three regions",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 10}
            pc.sketch.circle{sketch = s, x = 10, y = 0, radius = 10}
            local regions = pc.sketch.regions{sketch = s}
            assert(#regions == 3)
            local lens = 2 * 100 * math.acos(0.5) - 5 * math.sqrt(300)
            local found = false
            for _, r in ipairs(regions) do
              assert(r.default, "every region is material")
              if math.abs(r.x - 5) < 1 then
                found = true
                assert(math.abs(r.area - lens) < 0.01 * lens, r.area)
              end
            end
            assert(found, "the lens sits between the centres")
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.wall_thickness",
            "How thin the sketch's closed profile gets, for printing",
        ))
        .optional(
            "minimum",
            ParamKind::Number,
            "The thinnest wall that prints, mm; the Sketcher preference when left out",
        )
        .returns(
            "{thinnest, where = {x, y}, minimum, thin, regions}: the thinnest wall in mm, \
             where it is, whether it is under the minimum, and each region's own",
        )
        .read_only()
        .note(
            "It reads the sketch's closed profile, and refuses a sketch with none (\"profile \
             is not closed\").",
        )
        .note(
            "`minimum` is the Sketcher preference when left out (0.8 mm unless changed); \
             `thin` is true when the thinnest wall is under it.",
        )
        .see_also("sketch.status")
        .example(
            "A frame with 1 mm walls",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            pc.sketch.rect{sketch = s, x = 1, y = 1, width = 18, height = 8}
            local wall = pc.sketch.wall_thickness{sketch = s, minimum = 2.5}
            assert(math.abs(wall.thinnest - 1) < 1e-4, "the frame's wall is 1 mm")
            assert(wall.thin and wall.minimum == 2.5)
            assert(not pc.sketch.wall_thickness{sketch = s, minimum = 0.5}.thin)
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.status",
            "How constrained the sketch is, and what conflicts",
        ))
        .returns("{dof, solved, redundant, conflicting}")
        .read_only()
        .note(
            "`dof` is the freedom left: 0 is fully constrained. A free point has 2, a free \
             circle 3 (its centre and radius); a line's freedom is its two end points'.",
        )
        .note(
            "`solved` false with ids in `conflicting` means constraints contradict; every \
             constraint taking part is listed, not only the newest. `redundant` lists those \
             that say again what others say; the sketch still solves. While `solved` is \
             false, what is built from the sketch fails at `pc.doc.rebuild()`.",
        )
        .see_also("sketch.constrain")
        .see_also("sketch.constraints")
        .example(
            "A circle constrained, then over-constrained",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local c = pc.sketch.circle{sketch = s, x = 3, y = 4, radius = 5}
            assert(pc.sketch.status{sketch = s}.dof == 3, "a centre that moves and a radius")
            pc.sketch.constrain{sketch = s, kind = "radius", items = {c}, value = 5}
            local centre = pc.sketch.geometry{sketch = s}[1].id
            pc.sketch.constrain{sketch = s, kind = "lock", items = {centre}}
            local status = pc.sketch.status{sketch = s}
            assert(status.dof == 0 and status.solved and #status.conflicting == 0)
            pc.sketch.constrain{sketch = s, kind = "diameter", items = {c}, value = 12}
            status = pc.sketch.status{sketch = s}
            assert(not status.solved and #status.conflicting == 2, "radius 5 and diameter 12")
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.delete",
            "Delete elements or constraints, and what depends on them",
        ))
        .param("items", ParamKind::List, "Element or constraint ids")
        .note(
            "Deleting a point takes the curves that end or centre on it and their \
             constraints. Deleting a curve leaves its end points behind as loose points.",
        )
        .note(
            "A constraint's id deletes only that constraint. The origin and the axes are \
             passed over; an id the sketch does not have is refused.",
        )
        .see_also("sketch.construction")
        .see_also("sketch.repair")
        .example(
            "A corner deleted with the two lines on it",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local corner = pc.sketch.point{sketch = s, x = 20, y = 10}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            assert(#pc.sketch.constraints{sketch = s} == 4)
            pc.sketch.delete{sketch = s, items = {corner}}
            local lines = 0
            for _, e in ipairs(pc.sketch.geometry{sketch = s}) do
              if e.kind == "line" then lines = lines + 1 end
            end
            assert(lines == 2, "the two lines ending on the corner went with it")
            assert(#pc.sketch.constraints{sketch = s} == 2, "and the constraints on them")
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.construction",
            "Make elements construction geometry, or normal again",
        ))
        .param("items", ParamKind::List, "Element ids")
        .optional("on", ParamKind::Bool, "true (the default) or false")
        .note(
            "Construction geometry is left out of profiles: a construction circle inside an \
             outline cuts no hole. Constraints on it still hold.",
        )
        .note(
            "`sketch.draw` makes construction geometry from the start with `construction = true`.",
        )
        .see_also("sketch.draw")
        .see_also("sketch.delete")
        .example(
            "A circle that guides, then cuts",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local guide = pc.sketch.circle{sketch = s, x = 10, y = 5, radius = 3}
            pc.sketch.construction{sketch = s, items = {guide}}
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - 200) < 1e-3, "a construction circle cuts no hole")
            pc.sketch.construction{sketch = s, items = {guide}, on = false}
            assert(#pc.doc.rebuild() == 0)
            m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs(m.volume - (200 - math.pi * 9)) < 0.01, m.volume)
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.internal_geometry",
            "Show or hide curves' internal geometry: an ellipse's axes and foci, a parabola's \
             or hyperbola's axis and focus, a B-spline's control polygon, as construction held \
             to its curve",
        ))
        .param(
            "items",
            ParamKind::List,
            "The curves, or pieces of their internal geometry",
        )
        .optional(
            "show",
            ParamKind::Bool,
            "true makes what is missing, false takes away the pieces nothing else holds; \
             left out, it shows when a piece is missing and hides otherwise",
        )
        .returns("{shown, elements}: whether it showed, and what it made or took away")
        .note(
            "Without `show` it switches: the same call twice makes the pieces and takes them \
             away again. Give `show = true` to be sure they are there.",
        )
        .note(
            "An ellipse gets its major and minor axes with their ends and its two foci, all \
             construction held to it. Items naming no ellipse, parabola, hyperbola or \
             B-spline are refused.",
        )
        .see_also("sketch.draw")
        .see_also("sketch.constrain")
        .example(
            "An ellipse's axes and foci shown and hidden",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local made = pc.sketch.draw{sketch = s, tool = "ellipse", points = {{0, 0}, {10, 0}, {0, 4}}}
            local ellipse = made.elements[#made.elements]
            local shown = pc.sketch.internal_geometry{sketch = s, items = {ellipse}}
            assert(shown.shown and #shown.elements == 8, "two axes with their ends, and two foci")
            for _, e in ipairs(pc.sketch.geometry{sketch = s}) do
              if e.kind == "line" then assert(e.construction) end
            end
            local hidden = pc.sketch.internal_geometry{sketch = s, items = {ellipse}}
            assert(not hidden.shown and #pc.sketch.geometry{sketch = s} == 2, "the same call again takes them away")
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.section_view",
            "Cut away everything on the viewer's side of the sketch plane while it is edited",
        ))
        .optional("on", ParamKind::Bool, "true (the default) or false")
        .note(
            "A view setting only: nothing built from the sketch changes, and it shows only \
             while the sketch is open for editing in the window.",
        )
        .note("The sketch's data carries `section_view = true` while it is on.")
        .example(
            "The setting kept on the sketch",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.section_view{sketch = s}
            assert(pc.doc.feature{id = s}.fields.section_view == true)
            pc.sketch.section_view{sketch = s, on = false}
            assert(pc.doc.feature{id = s}.fields.section_view == nil)
            "#,
        ),
    );
    context.register_command(
        sketch(CommandSpec::new(
            "sketch.remove_axis_alignment",
            "Turn the horizontal and vertical constraints of lines into parallel and \
             perpendicular ones among them, so the group keeps its shape and turns as a whole",
        ))
        .param("items", ParamKind::List, "The lines")
        .returns("how many constraints changed")
        .note(
            "It returns how many horizontal and vertical constraints went; one fewer \
             parallel or perpendicular constraint takes their place, so the group gains the \
             freedom to turn. Lines with none answer 0.",
        )
        .see_also("sketch.constrain")
        .see_also("sketch.status")
        .example(
            "A rectangle freed to turn",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local sides = pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            assert(pc.sketch.status{sketch = s}.dof == 4)
            assert(pc.sketch.remove_axis_alignment{sketch = s, items = sides} == 4)
            for _, c in ipairs(pc.sketch.constraints{sketch = s}) do
              assert(c.kind == "Parallel" or c.kind == "Perpendicular", c.kind)
            end
            assert(pc.sketch.status{sketch = s}.dof == 5, "still a rectangle, free to turn")
            "#,
        ),
    );
}

/// The arguments that say where a new sketch goes.
fn placing(spec: CommandSpec) -> CommandSpec {
    spec.optional(
        "body",
        ParamKind::Id,
        "The body it belongs to; the selected body, else a new one",
    )
    .optional("plane", ParamKind::String, "XY (the default), XZ or YZ")
    .optional(
        "offset",
        ParamKind::Number,
        "How far along the plane's normal it sits",
    )
    .optional("name", ParamKind::String, "Its name in the tree")
    .optional(
        "attachment",
        ParamKind::Any,
        "Attached as a datum plane is, the attachment as a datum keeps it (doc.feature \
         on a datum shows it): {Face = {face = {point = {x, y, z}, normal = {x, y, z}}}}, \
         {ThreePoints = {points = {{At = {point = {x, y, z}}}, ...}}} and the like, faces \
         and edges on the body's own solid; the sketch follows what it stands on. \
         design.datum makes the same from plainer arguments, and `on` takes that datum",
    )
    .optional(
        "attachment_offset",
        ParamKind::Any,
        "The attachment's offset, as a datum's",
    )
    .optional(
        "on",
        ParamKind::Id,
        "A datum plane, or a coordinate system whose XY, XZ or YZ plane (see plane) it takes",
    )
    .optional(
        "normal",
        ParamKind::List,
        "A plane of its own instead: its normal as {x, y, z}",
    )
    .optional(
        "origin",
        ParamKind::List,
        "With normal: where the plane's origin sits, {x, y, z}",
    )
    .optional(
        "x_axis",
        ParamKind::List,
        "With normal: the sketch's X direction, {x, y, z}",
    )
}

/// Run command `id`, or say it is not one of these.
pub fn run(id: &str, args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    if id == "sketch.new" {
        let made_by = match a.opt_string("generator")? {
            Some(kind) => Some(crate::generator::Generator::named(kind).ok_or_else(|| {
                CommandError::bad("generator", "must be gear, sprocket, shaft or keyway")
            })?),
            None => None,
        };
        let name = match (a.opt_string("name")?, &made_by) {
            (Some(name), _) => name.to_string(),
            (None, Some(made_by)) => {
                crate::SketchWorkbench::next_generated_name(ctx.document, made_by.base_name())
            }
            (None, None) => crate::SketchWorkbench::next_sketch_name(ctx.document),
        };
        let id = add_sketch(&a, ctx, Sketch::new(name), made_by)?;
        return Ok(json!(id.0.to_string()));
    }
    if id == "sketch.import_dxf" {
        return import_dxf(&a, ctx);
    }
    if id == "sketch.image" {
        return add_image(&a, ctx);
    }
    let sketch_id = FeatureId(a.id("sketch")?);
    let mut feature = load(ctx, sketch_id)?;
    // What the sketch reads as is what builds: its formulas' values in,
    // solved.
    if matches!(
        id,
        "sketch.geometry" | "sketch.constraints" | "sketch.status" | "sketch.regions"
    ) {
        let evaluated = crate::stored_sketch(ctx.document, sketch_id).unwrap_or(feature);
        return Ok(match id {
            "sketch.geometry" => describe(&evaluated.sketch),
            "sketch.constraints" => describe_constraints(&evaluated.sketch),
            "sketch.regions" => describe_regions(&evaluated.sketch)?,
            _ => status(&evaluated.sketch),
        });
    }
    match id {
        "sketch.mirror_sketch" => {
            return mirror_sketch(ctx.document, sketch_id).map(|id| json!(id.0.to_string()));
        }
        "sketch.merge" => {
            let with = feature_ids(args.get("with"), "with")?;
            return merge(ctx.document, sketch_id, &with).map(|id| json!(id.0.to_string()));
        }
        "sketch.set_image" => {
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
                let placed = place_image(&a, feature.sketch.images[at].clone())?;
                feature.sketch.images[at] = placed;
            }
            return save(ctx, sketch_id, feature, Value::Null);
        }
        "sketch.attachment" => {
            let Some(mut support) = feature.support.clone() else {
                return Err(CommandError::bad("sketch", "is not attached to a datum"));
            };
            if let Some(v) = a.opt_number("offset")? {
                support.offset = v;
            }
            if let Some(v) = args.get("shift").filter(|v| !v.is_null()) {
                let pair = v
                    .as_array()
                    .filter(|p| p.len() == 2)
                    .and_then(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
                    .ok_or_else(|| CommandError::bad("shift", "must be {x, y}"))?;
                support.shift = pair;
            }
            if let Some(v) = a.opt_number("turn")? {
                support.turn = v;
            }
            let values = ctx
                .document
                .feature_values(support.datum)
                .cloned()
                .ok_or_else(|| CommandError::failed("the datum is gone"))?;
            let plane = support
                .plane_from(&values)
                .ok_or_else(|| CommandError::failed("the datum is not a plane"))?;
            feature.plane = plane;
            feature.sketch.plane = plane;
            feature.support = Some(support);
            return save(ctx, sketch_id, feature, Value::Null);
        }
        "sketch.external_from" => {
            let from = FeatureId(a.id("from")?);
            if from == sketch_id {
                return Err(CommandError::bad("from", "must be another sketch"));
            }
            let sources = reference_sources(ctx.document, from)
                .ok_or_else(|| CommandError::bad("from", "must be a sketch or a datum"))?;
            let sources = counting(&a, sources)?;
            let before = ids_of(&feature.sketch);
            let placed = crate::placed_plane(
                &feature.plane,
                &crate::sketch_placement(ctx.document, sketch_id),
            );
            add_external(ctx, &placed, &mut feature.sketch, &sources)
                .map_err(CommandError::failed)?;
            let made = made_since(&feature.sketch, &before);
            return save(ctx, sketch_id, feature, made);
        }
        "sketch.external_defining" => {
            let items = ids(args.get("items"), "items", &feature.sketch)?;
            // The points of an external curve go with it: the list
            // `sketch.external` answers with is taken as it comes.
            let on_external: std::collections::HashSet<Uuid> = feature
                .sketch
                .geometry
                .iter()
                .filter(|g| feature.sketch.external.contains_key(&g.id()))
                .flat_map(Sketch::curve_point_ids)
                .collect();
            let items: Vec<Uuid> = items
                .into_iter()
                .filter(|i| feature.sketch.external.contains_key(i) || !on_external.contains(i))
                .collect();
            if let Some(bad) = items
                .iter()
                .find(|i| !feature.sketch.external.contains_key(i))
            {
                return Err(CommandError::bad(
                    "items",
                    format!("{bad} is not external geometry"),
                ));
            }
            set_external_defining(
                &mut feature.sketch,
                &items,
                a.opt_bool("on")?.unwrap_or(true),
            );
            return save(ctx, sketch_id, feature, Value::Null);
        }
        "sketch.solver_settings" => {
            if let Some(n) = a.opt_number("iterations")? {
                if n < 1.0 {
                    return Err(CommandError::bad("iterations", "must be at least 1"));
                }
                feature.sketch.solver.max_iterations = n as u32;
            }
            if let Some(t) = a.opt_number("tolerance")? {
                if !(t > 0.0 && t < 1.0) {
                    return Err(CommandError::bad(
                        "tolerance",
                        "must be above 0 and below 1",
                    ));
                }
                feature.sketch.solver.tolerance = t;
            }
            crate::solver::solve(&mut feature.sketch);
            return save(ctx, sketch_id, feature, Value::Null);
        }
        "sketch.repair" => {
            let tolerance = a.opt_number("tolerance")?.unwrap_or(0.01);
            let done = crate::repair::repair(&mut feature.sketch, tolerance);
            crate::solver::solve(&mut feature.sketch);
            let words = done.describe();
            return save(ctx, sketch_id, feature, Value::Null).map(|_| json!(words));
        }
        "sketch.restore" => {
            let data = args
                .get("data")
                .ok_or_else(|| CommandError::bad("data", "is needed"))?;
            let restored = SketchFeature::from_json(data)
                .map_err(|e| CommandError::bad("data", e.to_string()))?;
            ctx.document
                .set_feature_dependencies(sketch_id, restored.dependencies());
            return save(ctx, sketch_id, restored, Value::Null);
        }
        "sketch.set_plane" => {
            let plane = custom_plane(&a)?;
            feature.plane = plane;
            feature.sketch.plane = plane;
            // A plane set outright leaves the datum the sketch followed.
            if feature.support.take().is_some() {
                ctx.document
                    .set_feature_dependencies(sketch_id, feature.dependencies());
            }
            return save(ctx, sketch_id, feature, Value::Null);
        }
        // A view setting: nothing built from the sketch changes.
        "sketch.section_view" => {
            feature.set_section(a.opt_bool("on")?.unwrap_or(true));
            ctx.document
                .update_feature_data(sketch_id, feature.to_json())
                .map_err(|e| CommandError::failed(e.to_string()))?;
            return Ok(Value::Null);
        }
        "sketch.carbon_copy" => {
            let from = FeatureId(a.id("from")?);
            let before = ids_of(&feature.sketch);
            carbon_copy(ctx.document, sketch_id, &mut feature.sketch, from)
                .map_err(CommandError::failed)?;
            let made = made_since(&feature.sketch, &before);
            return save(ctx, sketch_id, feature, made);
        }
        "sketch.intersection" => {
            let faces = counting(&a, section_sources(args.get("faces"))?)?;
            let before = ids_of(&feature.sketch);
            let placed = crate::placed_plane(
                &feature.plane,
                &crate::sketch_placement(ctx.document, sketch_id),
            );
            add_external(ctx, &placed, &mut feature.sketch, &faces)
                .map_err(CommandError::failed)?;
            let made = made_since(&feature.sketch, &before);
            return save(ctx, sketch_id, feature, made);
        }
        "sketch.external" => {
            let edges = counting(&a, external_sources(args.get("edges"))?)?;
            let before = ids_of(&feature.sketch);
            let placed = crate::placed_plane(
                &feature.plane,
                &crate::sketch_placement(ctx.document, sketch_id),
            );
            let added = add_external(ctx, &placed, &mut feature.sketch, &edges)
                .map_err(CommandError::failed)?;
            if added == 0 {
                return Err(CommandError::failed("no edge could be projected"));
            }
            let made = made_since(&feature.sketch, &before);
            return save(ctx, sketch_id, feature, made);
        }
        _ => {}
    }
    let sketch = &mut feature.sketch;
    let answer = match id {
        "sketch.point" => {
            let p = point_at(sketch, vec(a.number("x")?, a.number("y")?));
            json!(p.to_string())
        }
        "sketch.line" => {
            let from = vec(a.number("x1")?, a.number("y1")?);
            let to = vec(a.number("x2")?, a.number("y2")?);
            json!(line(sketch, from, to)?.to_string())
        }
        "sketch.polyline" => {
            let points = points(args.get("points"))?;
            let closed = a.opt_bool("closed")?.unwrap_or(false);
            json!(polyline(sketch, &points, closed)?)
        }
        "sketch.rect" => {
            let (x, y) = (a.number("x")?, a.number("y")?);
            let (w, h) = (a.number("width")?, a.number("height")?);
            if w == 0.0 || h == 0.0 {
                return Err(CommandError::failed(
                    "a rectangle needs a width and a height",
                ));
            }
            let corners = [vec(x, y), vec(x + w, y), vec(x + w, y + h), vec(x, y + h)];
            json!(polyline(sketch, &corners, true)?)
        }
        "sketch.circle" => {
            let radius = positive(&a, "radius")?;
            let centre = point_at(sketch, vec(a.number("x")?, a.number("y")?));
            let id = sketch.add_geometry(GeometryElement::Circle(Circle::new(centre, radius)));
            json!(id.to_string())
        }
        "sketch.arc" => {
            let radius = positive(&a, "radius")?;
            let (x, y) = (a.number("x")?, a.number("y")?);
            let at = |deg: f64| {
                let t = deg.to_radians();
                vec(x + radius * t.cos(), y + radius * t.sin())
            };
            let (start, end) = (a.number("start")?, a.number("end")?);
            let centre = point_at(sketch, vec(x, y));
            let s = point_at(sketch, at(start));
            let e = point_at(sketch, at(end));
            let id = sketch.add_geometry(GeometryElement::Arc(Arc::new(centre, s, e, radius)));
            json!(id.to_string())
        }
        "sketch.constrain" => {
            let kind = a.string("kind")?;
            let items = ids(args.get("items"), "items", sketch)?;
            let value = a.opt_number("value")?;
            let remove_redundant = a.opt_bool("remove_redundant")?.unwrap_or(false);
            let made = constrain(sketch, kind, &items, value)?;
            if remove_redundant {
                remove_superseded(sketch, &made);
            }
            json!(made)
        }
        "sketch.set_value" => {
            let id = a.id("constraint")?;
            let value = a.number("value")?;
            let constraint = sketch
                .constraints
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or_else(|| CommandError::bad("constraint", "is not in this sketch"))?;
            if crate::sketch::dimension_value(&constraint.kind).is_none() {
                return Err(CommandError::bad("constraint", "is not a dimension"));
            }
            constraint.kind = crate::sketch::with_dimension_value(&constraint.kind, value);
            if let Some(driving) = a.opt_bool("driving")? {
                constraint.driving = driving;
            }
            Value::Null
        }
        "sketch.delete" => {
            let items = ids(args.get("items"), "items", sketch)?;
            delete_items(sketch, &items);
            Value::Null
        }
        "sketch.draw" => {
            let (elements, constraints) = draw(sketch, &a, args)?;
            json!({"elements": elements, "constraints": constraints})
        }
        "sketch.array" => {
            let items: std::collections::HashSet<Uuid> = ids(args.get("items"), "items", sketch)?
                .into_iter()
                .collect();
            let count = |name: &str| -> Result<u32, CommandError> {
                let n = a.number(name)?;
                if n >= 1.0 {
                    Ok(n as u32)
                } else {
                    Err(CommandError::bad(name, "must be at least 1"))
                }
            };
            let before = ids_of(sketch);
            let effect = crate::tools::array(
                sketch,
                &items,
                count("rows")?,
                count("cols")?,
                a.number("dx")?,
                a.number("dy")?,
                a.opt_bool("linked")?.unwrap_or(false),
            );
            if !effect.changed {
                return Err(CommandError::failed(
                    "an array needs elements and at least two rows or columns",
                ));
            }
            made_since(sketch, &before)
        }
        "sketch.text" => {
            let mut spec = crate::text::TextSpec {
                text: a.string("text")?.to_string(),
                ..Default::default()
            };
            text_options(&a, &mut spec)?;
            crate::text::load_font(ctx.document, &mut spec).map_err(CommandError::failed)?;
            let at = points(Some(&json!([args
                .get("at")
                .cloned()
                .unwrap_or(Value::Null)])))
            .map_err(|_| CommandError::bad("at", "must be {x, y}"))?[0];
            let block = crate::text::add(sketch, at, &spec).map_err(CommandError::failed)?;
            let point = sketch
                .texts
                .iter()
                .find(|b| b.id == block)
                .map(|b| b.anchor)
                .unwrap_or_default();
            json!({"text": block.to_string(), "point": point.to_string()})
        }
        "sketch.text_edit" => {
            let text = args
                .get("block")
                .and_then(Value::as_str)
                .and_then(|t| Uuid::parse_str(t).ok())
                .ok_or_else(|| CommandError::bad("block", "must be a text block's id"))?;
            let block = sketch
                .texts
                .iter()
                .find(|b| b.id == text || b.anchor == text)
                .ok_or_else(|| CommandError::bad("block", "is no text block"))?;
            let id = block.id;
            let mut spec = crate::text::TextSpec::of(block);
            if let Some(t) = a.opt_string("text")? {
                spec.text = t.to_string();
            }
            text_options(&a, &mut spec)?;
            crate::text::load_font(ctx.document, &mut spec).map_err(CommandError::failed)?;
            crate::text::change(sketch, id, &spec).map_err(CommandError::failed)?;
            Value::Null
        }
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
        "sketch.set_constraint" => {
            let items = ids(args.get("items"), "items", sketch)?;
            let flags = ConstraintFlags {
                driving: a.opt_bool("driving")?,
                active: a.opt_bool("active")?,
                parked: a.opt_bool("parked")?,
            };
            for id in &items {
                let Some(c) = sketch.constraints.iter().find(|c| c.id == *id) else {
                    return Err(CommandError::bad(
                        "items",
                        format!("has {id}, which is not a constraint"),
                    ));
                };
                if flags.driving == Some(false) && crate::sketch::dimension_value(&c.kind).is_none()
                {
                    return Err(CommandError::bad(
                        "driving",
                        format!("cannot be false for {id}: only a dimension can be a reference"),
                    ));
                }
            }
            let switch = match a.opt_string("as")? {
                None => None,
                Some("radius") => Some(false),
                Some("diameter") => Some(true),
                Some(_) => return Err(CommandError::bad("as", "must be radius or diameter")),
            };
            set_constraints(sketch, &items, flags);
            if let Some(diameter) = switch {
                let circular = |id: &Uuid| {
                    sketch.constraints.iter().any(|c| {
                        c.id == *id
                            && matches!(
                                c.kind,
                                ConstraintKind::Radius { .. } | ConstraintKind::Diameter { .. }
                            )
                    })
                };
                if let Some(id) = items.iter().find(|id| !circular(id)) {
                    return Err(CommandError::bad(
                        "as",
                        format!("applies to radius and diameter dimensions, not {id}"),
                    ));
                }
                for id in &items {
                    switch_dimension(ctx.document, sketch_id, sketch, *id, diameter);
                }
            }
            Value::Null
        }
        "sketch.paste" => {
            let clip: Sketch =
                serde_json::from_value(args.get("clipboard").cloned().unwrap_or(Value::Null))
                    .map_err(|e| CommandError::bad("clipboard", e.to_string()))?;
            let by = points(Some(&json!([args
                .get("by")
                .cloned()
                .unwrap_or(Value::Null)])))
            .map_err(|_| CommandError::bad("by", "must be {x, y}"))?[0];
            let before = ids_of(sketch);
            paste(sketch, &clip, by);
            made_since(sketch, &before)
        }
        "sketch.drag" => {
            let items = ids(args.get("items"), "items", sketch)?;
            let by = points(Some(&json!([args
                .get("by")
                .cloned()
                .unwrap_or(Value::Null)])))
            .map_err(|_| CommandError::bad("by", "must be {x, y}"))?[0];
            let targets = crate::step::drag_targets(sketch, &items);
            crate::step::drag(sketch, &targets, by);
            let held: Vec<Uuid> = targets.iter().map(|(id, _)| *id).collect();
            crate::solver::solve_holding(sketch, &held);
            Value::Null
        }
        "sketch.internal_geometry" => {
            let items = ids(args.get("items"), "items", sketch)?;
            if crate::internal::curves_of(sketch, &items).is_empty() {
                return Err(CommandError::bad(
                    "items",
                    "names no ellipse, parabola, hyperbola or B-spline",
                ));
            }
            let (shown, changed) = crate::internal::toggle(sketch, &items, a.opt_bool("show")?);
            let changed: Vec<String> = changed.iter().map(Uuid::to_string).collect();
            json!({"shown": shown, "elements": changed})
        }
        "sketch.remove_axis_alignment" => {
            let items: std::collections::HashSet<Uuid> = ids(args.get("items"), "items", sketch)?
                .into_iter()
                .collect();
            json!(crate::tools::remove_axis_alignment(sketch, &items))
        }
        "sketch.construction" => {
            let on = a.opt_bool("on")?.unwrap_or(true);
            for id in ids(args.get("items"), "items", sketch)? {
                sketch.set_construction(id, on);
            }
            Value::Null
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
    let asset = ctx.document.add_asset_with_data(
        core_document::AssetReference::new(
            format!("assets/reference-{}.{extension}", Uuid::new_v4()),
            core_document::AssetType::Other,
            json!({"kind": "reference image", "file": path.file_name().map(|n| n.to_string_lossy())}),
        ),
        bytes,
    );
    let image = place_image(
        a,
        crate::sketch::ReferenceImage {
            id: Uuid::new_v4(),
            asset,
            center: Vec2D::new(0.0, 0.0),
            width: DEFAULT_IMAGE_WIDTH_MM,
            angle_deg: 0.0,
            opacity: 0.5,
        },
    )?;
    let is_sketch = |id: FeatureId| {
        ctx.document
            .get_feature_meta(id)
            .is_some_and(|n| n.workbench_id.as_str() == "wb.sketch")
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

/// A sketch of the DXF drawing at `path`, placed as `sketch.new` places
/// one and named after the file unless `name` says otherwise.
fn import_dxf(a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let path = std::path::Path::new(a.string("path")?);
    let given_scale = a.opt_number("scale")?;
    if given_scale.is_some_and(|s| !(s > 0.0 && s.is_finite())) {
        return Err(CommandError::bad("scale", "must be more than zero"));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| CommandError::bad("path", format!("could not be read: {e}")))?;
    // DXF text is often Latin-1 in its strings; the geometry is ASCII.
    let text = String::from_utf8_lossy(&bytes);
    let kernel = ctx
        .kernel
        .ok_or_else(|| CommandError::failed("there is no kernel to read the drawing"))?;
    let drawing = kernel
        .read_dxf(&text)
        .map_err(|e| CommandError::failed(e.to_string()))?;
    let name = match a.opt_string("name")? {
        Some(name) => name.to_string(),
        None => path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| crate::SketchWorkbench::next_sketch_name(ctx.document)),
    };
    // The scale given, else the drawing's own unit, else millimetres.
    let scale = given_scale.or(drawing.unit_mm).unwrap_or(1.0);
    let mut sketch = Sketch::new(name);
    let added = crate::dxf::add_drawing(&mut sketch, &drawing, scale);
    if added.curves() == 0 {
        return Err(CommandError::failed(
            "the drawing has no curves to bring in",
        ));
    }
    crate::solver::solve(&mut sketch);
    let id = add_sketch(a, ctx, sketch, None)?;
    let mut parts: Vec<String> = [
        (added.lines, "lines"),
        (added.arcs, "arcs"),
        (added.circles, "circles"),
        (added.ellipses, "ellipses"),
    ]
    .iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}"))
    .collect();
    if added.construction > 0 {
        parts.push(format!("{} of them construction", added.construction));
    }
    if added.splines > 0 {
        parts.push(format!("{} spline(s) as lines through them", added.splines));
    }
    ctx.log_info(format!(
        "Imported {} at {scale} mm a unit: {}",
        path.display(),
        parts.join(", ")
    ));
    Ok(json!(id.0.to_string()))
}

/// Add `sketch` as a new sketch feature where `a` places it: on a base
/// plane, a datum or a plane of its own, in the body given or selected,
/// else a new one.
fn add_sketch(
    a: &Args,
    ctx: &mut WorkbenchRuntimeContext,
    mut sketch: Sketch,
    made_by: Option<crate::generator::Generator>,
) -> Result<FeatureId, CommandError> {
    let mut plane = match a.opt_string("plane")?.unwrap_or("XY") {
        p if p.eq_ignore_ascii_case("XY") => SketchPlane::xy(),
        p if p.eq_ignore_ascii_case("XZ") => SketchPlane::xz(),
        p if p.eq_ignore_ascii_case("YZ") => SketchPlane::yz(),
        _ => return Err(CommandError::bad("plane", "must be XY, XZ or YZ")),
    };
    if a.has("normal") {
        plane = custom_plane(a)?;
    }
    let mut datum_body = None;
    let offset = a.opt_number("offset")?.unwrap_or(0.0);
    let mut support = None;
    if let Some(on) = a.opt_id("on")? {
        let (on_datum, body) = datum_support(ctx, FeatureId(on), a.opt_string("plane")?, offset)?;
        plane = on_datum.1;
        support = Some(on_datum.0);
        datum_body = body;
    } else {
        for (o, n) in plane.origin.iter_mut().zip(plane.normal) {
            *o += n * offset;
        }
    }
    let body = match a.opt_id("body")?.or(datum_body.map(|b| b.0)) {
        Some(id) => {
            let body = BodyId(id);
            if !ctx.document.bodies().iter().any(|b| b.id == body) {
                return Err(CommandError::bad("body", "is not a body of this document"));
            }
            body
        }
        None => match ctx.selected_body_id {
            Some(id) => BodyId(id),
            None => ctx.document.create_body(None),
        },
    };
    // Attached by a mode: the plane is where the attachment puts it on
    // the body, and follows it.
    let attached = match args_value(a, "attachment") {
        Some(value) => {
            let mut value = value.clone();
            following(&mut value);
            let attachment: core_document::DatumAttachment = serde_json::from_value(value)
                .map_err(|e| CommandError::bad("attachment", e.to_string()))?;
            let offset = match args_value(a, "attachment_offset") {
                Some(v) => serde_json::from_value(v.clone())
                    .map_err(|e| CommandError::bad("attachment_offset", e.to_string()))?,
                None => core_document::AttachmentOffset::default(),
            };
            let attached = settle_attached(ctx, body, attachment, offset)
                .map_err(|e| CommandError::bad("attachment", e))?;
            plane = attached.plane();
            Some(attached)
        }
        None => None,
    };
    sketch.plane = plane;
    let name = sketch.name.clone();
    let mut feature = SketchFeature::new(sketch, plane);
    feature.support = support;
    feature.attached = attached;
    if let Some(made_by) = made_by {
        feature.generator = Some(made_by);
        crate::generator::regenerate(&mut feature).map_err(CommandError::failed)?;
    }
    ctx.document
        .add_feature_in_body(feature, name, Some(body))
        .map_err(|e| CommandError::failed(e.to_string()))
}

/// Every face and edge an attachment given to a command names is on the
/// sketch's own body, so the sketch follows it, unless the script says
/// otherwise: `follows` set on each anchor (a point with a normal or a
/// direction) that does not give it.
fn following(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let anchor = map.contains_key("point")
                && (map.contains_key("normal") || map.contains_key("direction"));
            if anchor && !map.contains_key("follows") {
                map.insert("follows".into(), Value::Bool(true));
            }
            for child in map.values_mut() {
                following(child);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(following),
        _ => {}
    }
}

/// An argument's raw value, when given and not nil.
fn args_value<'a>(a: &'a Args, name: &str) -> Option<&'a Value> {
    a.0.get(name).filter(|v| !v.is_null())
}

/// An attachment on `body`, filled in from its solid where there is one,
/// as a sketch keeps it.
pub(crate) fn settle_attached(
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    attachment: core_document::DatumAttachment,
    offset: core_document::AttachmentOffset,
) -> Result<crate::feature::AttachedSupport, String> {
    let mut datum = crate::feature::AttachedSupport { attachment, offset }.datum();
    core_document::attach::settle(ctx, body, &mut datum)?;
    Ok(crate::feature::AttachedSupport {
        attachment: datum.attachment,
        offset: datum.offset,
    })
}

fn load(ctx: &WorkbenchRuntimeContext, id: FeatureId) -> Result<SketchFeature, CommandError> {
    let data = ctx
        .document
        .get_feature_data(id)
        .ok_or_else(|| CommandError::bad("sketch", "is not a feature of this document"))?;
    SketchFeature::from_json(data)
        .map_err(|why| CommandError::bad("sketch", format!("is not a sketch: {why}")))
}

fn vec(x: f64, y: f64) -> Vec2D {
    Vec2D::new(x, y)
}

fn positive(a: &Args, name: &str) -> Result<f64, CommandError> {
    let value = a.number(name)?;
    if value > 0.0 {
        Ok(value)
    } else {
        Err(CommandError::bad(name, "must be more than zero"))
    }
}

/// The point at `at`: one the sketch has there already, else a new one.
fn point_at(sketch: &mut Sketch, at: Vec2D) -> Uuid {
    let existing = sketch.geometry.iter().find_map(|g| match g {
        GeometryElement::Point(p) if (p.position - at).to_glam().length() < 1e-5 => Some(p.id),
        _ => None,
    });
    existing.unwrap_or_else(|| sketch.add_geometry(GeometryElement::Point(Point::new(at))))
}

fn line(sketch: &mut Sketch, from: Vec2D, to: Vec2D) -> Result<Uuid, CommandError> {
    if (to - from).to_glam().length() < 1e-5 {
        return Err(CommandError::failed("a line needs two different ends"));
    }
    let (a, b) = (point_at(sketch, from), point_at(sketch, to));
    Ok(sketch.add_geometry(GeometryElement::Line(Line::new(a, b))))
}

fn polyline(
    sketch: &mut Sketch,
    points: &[Vec2D],
    closed: bool,
) -> Result<Vec<String>, CommandError> {
    if points.len() < 2 {
        return Err(CommandError::bad("points", "needs at least two points"));
    }
    let mut segments: Vec<(Vec2D, Vec2D)> = points.windows(2).map(|p| (p[0], p[1])).collect();
    if closed && points.len() > 2 {
        segments.push((points[points.len() - 1], points[0]));
    }
    let mut ids = Vec::new();
    for (from, to) in segments {
        let id = line(sketch, from, to)?;
        // Level or upright as given, it stays so, as the line tool holds it.
        if (to.y - from.y).abs() <= 1e-9 && (to.x - from.x).abs() > 1e-9 {
            sketch.add_constraint(crate::sketch::ConstraintKind::Horizontal { element: id });
        } else if (to.x - from.x).abs() <= 1e-9 && (to.y - from.y).abs() > 1e-9 {
            sketch.add_constraint(crate::sketch::ConstraintKind::Vertical { element: id });
        }
        ids.push(id.to_string());
    }
    Ok(ids)
}

/// Where a sketch on a datum stands, and the plane that puts it there now.
type OnDatum = (DatumSupport, SketchPlane);

/// A sketch's place on datum `id`: a datum plane, or one of a coordinate
/// system's three (`which`, XY when left out), `offset` along its normal;
/// with the plane that puts it on now and the body the datum is in.
fn datum_support(
    ctx: &WorkbenchRuntimeContext,
    id: FeatureId,
    which: Option<&str>,
    offset: f64,
) -> Result<(OnDatum, Option<BodyId>), CommandError> {
    let not_a_plane = || CommandError::bad("on", "is not a datum plane or coordinate system");
    let node = ctx.document.get_feature_meta(id).ok_or_else(not_a_plane)?;
    let datum = DatumFeature::from_json(&node.data).map_err(|why| {
        CommandError::bad(
            "on",
            format!("is not a datum plane or coordinate system: {why}"),
        )
    })?;
    let plane = match datum.shape {
        DatumShape::Plane { .. } => None,
        DatumShape::CoordinateSystem { .. } => {
            let which = which.unwrap_or("XY");
            if !["XY", "XZ", "YZ"]
                .iter()
                .any(|p| p.eq_ignore_ascii_case(which))
            {
                return Err(CommandError::bad("plane", "must be XY, XZ or YZ"));
            }
            Some(which.to_ascii_uppercase())
        }
        _ => return Err(not_a_plane()),
    };
    let support = DatumSupport {
        datum: id,
        plane,
        offset,
        shift: [0.0, 0.0],
        turn: 0.0,
    };
    let values = ctx
        .document
        .feature_values(id)
        .cloned()
        .unwrap_or_else(|| node.data.clone());
    let at = support.plane_from(&values).ok_or_else(not_a_plane)?;
    Ok(((support, at), node.body))
}

/// A plane from `normal`, `origin` and `x_axis`, the last two optional.
fn custom_plane(a: &Args) -> Result<SketchPlane, CommandError> {
    let normal = unit(vector3(a.0.get("normal"), "normal")?, "normal")?;
    let origin = match a.0.get("origin") {
        Some(v) if !v.is_null() => vector3(Some(v), "origin")?,
        _ => [0.0; 3],
    };
    // Any direction square to the normal serves when none is given.
    let guess = if normal[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let wanted = match a.0.get("x_axis") {
        Some(v) if !v.is_null() => vector3(Some(v), "x_axis")?,
        _ => cross(guess, normal),
    };
    let along = dot(wanted, normal);
    let x_axis = unit(
        [
            wanted[0] - along * normal[0],
            wanted[1] - along * normal[1],
            wanted[2] - along * normal[2],
        ],
        "x_axis",
    )?;
    let y_axis = cross(normal, x_axis);
    let f = |v: [f64; 3]| v.map(|c| c);
    Ok(SketchPlane {
        origin: f(origin),
        normal: f(normal),
        x_axis: f(x_axis),
        y_axis: f(y_axis),
    })
}

fn vector3(value: Option<&Value>, name: &str) -> Result<[f64; 3], CommandError> {
    let bad = || CommandError::bad(name, "must be {x, y, z}");
    let v = match value {
        Some(Value::Array(v)) if v.len() == 3 => [v[0].as_f64(), v[1].as_f64(), v[2].as_f64()],
        Some(Value::Object(m)) => ["x", "y", "z"].map(|k| m.get(k).and_then(Value::as_f64)),
        _ => return Err(bad()),
    };
    Ok([
        v[0].ok_or_else(bad)?,
        v[1].ok_or_else(bad)?,
        v[2].ok_or_else(bad)?,
    ])
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(v: [f64; 3], name: &str) -> Result<[f64; 3], CommandError> {
    let length = dot(v, v).sqrt();
    if length < 1e-9 {
        return Err(CommandError::bad(
            name,
            "must not be zero, or along the normal",
        ));
    }
    Ok(v.map(|c| c / length))
}

/// A text's font, size, spacing and angle where `a` gives them.
fn text_options(a: &Args, spec: &mut crate::text::TextSpec) -> Result<(), CommandError> {
    if let Some(font) = a.opt_string("font")? {
        spec.font = font.to_string();
        spec.font_asset = None;
    }
    if let Some(asset) = a.opt_string("font_asset")? {
        spec.font_asset = Some(
            Uuid::parse_str(asset)
                .map_err(|_| CommandError::bad("font_asset", "must be an asset's id"))?,
        );
    }
    if let Some(size) = a.opt_number("size")? {
        if size <= 0.0 {
            return Err(CommandError::bad("size", "must be more than 0"));
        }
        spec.size = size;
    }
    if let Some(v) = a.opt_number("spacing")? {
        spec.spacing = v;
    }
    if let Some(v) = a.opt_number("angle")? {
        spec.angle = v;
    }
    Ok(())
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

/// Ids named in a list: element and constraint ids, and the names of the
/// sketch's origin and axes.
fn ids(value: Option<&Value>, name: &str, sketch: &Sketch) -> Result<Vec<Uuid>, CommandError> {
    let list = value
        .and_then(Value::as_array)
        .ok_or_else(|| CommandError::bad(name, "must be a list of ids"))?;
    list.iter()
        .map(|v| {
            let text = v
                .as_str()
                .ok_or_else(|| CommandError::bad(name, "must be a list of ids"))?;
            let id = match text {
                "origin" => crate::sketch::ORIGIN_ID,
                "x_axis" => crate::sketch::X_AXIS_ID,
                "y_axis" => crate::sketch::Y_AXIS_ID,
                other => Uuid::parse_str(other)
                    .map_err(|_| CommandError::bad(name, format!("has `{other}`, not an id")))?,
            };
            let known = sketch.get_geometry(id).is_some()
                || sketch.constraints.iter().any(|c| c.id == id)
                || [
                    crate::sketch::ORIGIN_ID,
                    crate::sketch::X_AXIS_ID,
                    crate::sketch::Y_AXIS_ID,
                ]
                .contains(&id);
            if known {
                Ok(id)
            } else {
                Err(CommandError::bad(
                    name,
                    format!("has {id}, which is not in this sketch"),
                ))
            }
        })
        .collect()
}

/// Store an edited sketch and answer `answer`.
fn save(
    ctx: &mut WorkbenchRuntimeContext,
    id: FeatureId,
    mut feature: SketchFeature,
    answer: Value,
) -> CommandResult {
    crate::solver::solve(&mut feature.sketch);
    ctx.document
        .update_feature_data(id, feature.to_json())
        .map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document.mark_feature_dirty(id);
    Ok(answer)
}

pub(crate) fn ids_of(
    sketch: &Sketch,
) -> (
    std::collections::HashSet<Uuid>,
    std::collections::HashSet<Uuid>,
) {
    (
        sketch.geometry.iter().map(GeometryElement::id).collect(),
        sketch.constraints.iter().map(|c| c.id).collect(),
    )
}

/// `{elements, constraints}`: what the sketch gained since `before`.
pub(crate) fn made_since(
    sketch: &Sketch,
    before: &(
        std::collections::HashSet<Uuid>,
        std::collections::HashSet<Uuid>,
    ),
) -> Value {
    let elements: Vec<String> = sketch
        .geometry
        .iter()
        .map(GeometryElement::id)
        .filter(|id| !before.0.contains(id))
        .map(|id| id.to_string())
        .collect();
    let constraints: Vec<String> = sketch
        .constraints
        .iter()
        .map(|c| c.id)
        .filter(|id| !before.1.contains(id))
        .map(|id| id.to_string())
        .collect();
    json!({"elements": elements, "constraints": constraints})
}

/// Sketch ids named in a list.
fn feature_ids(value: Option<&Value>, name: &str) -> Result<Vec<FeatureId>, CommandError> {
    value
        .and_then(Value::as_array)
        .ok_or_else(|| CommandError::bad(name, "must be a list of sketch ids"))?
        .iter()
        .map(|v| {
            v.as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .map(FeatureId)
                .ok_or_else(|| CommandError::bad(name, "must be a list of sketch ids"))
        })
        .collect()
}

/// Set the driving, active and parked flags of `items`, where given.
pub(crate) fn set_constraints(sketch: &mut Sketch, items: &[Uuid], flags: ConstraintFlags) {
    for c in &mut sketch.constraints {
        if items.contains(&c.id) {
            if let Some(driving) = flags.driving {
                c.driving = driving;
            }
            if let Some(active) = flags.active {
                c.active = active;
            }
            if let Some(parked) = flags.parked {
                c.parked = parked;
            }
        }
    }
}

/// The flags `sketch.set_constraint` sets; `None` leaves one as it is.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ConstraintFlags {
    pub driving: Option<bool>,
    pub active: Option<bool>,
    pub parked: Option<bool>,
}

/// Add `clip`'s geometry to `sketch`, moved by `by`.
pub(crate) fn paste(sketch: &mut Sketch, clip: &Sketch, by: Vec2D) -> usize {
    let all: std::collections::HashSet<Uuid> =
        clip.geometry.iter().map(GeometryElement::id).collect();
    crate::tools::copy_from(
        clip,
        sketch,
        &all,
        &crate::tools::Similarity::translation(glam::DVec2::new(by.x, by.y)),
    )
}

/// Turn constraint `id` into a diameter (or a radius) of the same circle,
/// its value converted. The factor its value was multiplied by; `None`
/// when it is no radius or diameter dimension. A dimension already of the
/// kind asked for stays as it is.
pub(crate) fn switch_circle_dimension(
    sketch: &mut Sketch,
    id: Uuid,
    diameter: bool,
) -> Option<f64> {
    let c = sketch.constraints.iter_mut().find(|c| c.id == id)?;
    let (kind, scale) = match (&c.kind, diameter) {
        (ConstraintKind::Radius { circle, radius }, true) => (
            ConstraintKind::Diameter {
                circle: *circle,
                diameter: radius * 2.0,
            },
            2.0,
        ),
        (ConstraintKind::Diameter { circle, diameter }, false) => (
            ConstraintKind::Radius {
                circle: *circle,
                radius: diameter * 0.5,
            },
            0.5,
        ),
        (ConstraintKind::Radius { .. }, false) | (ConstraintKind::Diameter { .. }, true) => {
            return Some(1.0);
        }
        _ => return None,
    };
    c.kind = kind;
    Some(scale)
}

/// [`switch_circle_dimension`] on sketch `sketch_id`'s `sketch`, the
/// formula on the dimension, if any, scaled with its value.
pub(crate) fn switch_dimension(
    document: &mut core_document::Document,
    sketch_id: FeatureId,
    sketch: &mut Sketch,
    id: Uuid,
    diameter: bool,
) -> Option<f64> {
    let scale = switch_circle_dimension(sketch, id, diameter)?;
    let key = id.to_string();
    if let Some(formula) = document.feature_formula(sketch_id, &key) {
        let scaled = scaled_formula(formula, scale);
        document
            .set_feature_formula(sketch_id, key, Some(scaled))
            .ok()?;
    }
    Some(scale)
}

/// `formula` giving `scale` times what it gave, undoing the opposite
/// switch rather than stacking on it.
pub(crate) fn scaled_formula(formula: &str, scale: f64) -> String {
    let formula = formula.trim();
    let (undo, apply) = if scale > 1.0 {
        (" / 2", " * 2")
    } else {
        (" * 2", " / 2")
    };
    if scale == 1.0 {
        return formula.to_string();
    }
    if let Some(inner) = formula
        .strip_suffix(undo)
        .and_then(|f| f.strip_prefix('('))
        .and_then(|f| f.strip_suffix(')'))
    {
        return inner.to_string();
    }
    format!("({formula}){apply}")
}

/// Sketch `id` with its plane where its body sits.
fn placed(document: &core_document::Document, id: FeatureId) -> Option<SketchFeature> {
    let mut feature = crate::stored_sketch(document, id)?;
    let placement = crate::sketch_placement(document, id);
    feature.plane = crate::placed_plane(&feature.plane, &placement);
    Some(feature)
}

/// Copy sketch `from`'s geometry into `target`, the geometry of sketch
/// `target_id`, mapped from its plane onto the target's (both where their
/// bodies sit). How many elements and constraints came.
pub(crate) fn carbon_copy(
    document: &core_document::Document,
    target_id: FeatureId,
    target: &mut Sketch,
    from: FeatureId,
) -> Result<(usize, usize), String> {
    let onto = placed(document, target_id).ok_or("the sketch is not in this document")?;
    let source = placed(document, from).ok_or("`from` is not a sketch of this document")?;
    if from == target_id {
        return Err("a sketch cannot copy itself".to_string());
    }
    let xf = crate::plane_map(&source.plane, &onto.plane)?;
    Ok(crate::copy_sketch_into(&source.sketch, target, &xf))
}

/// A new sketch on `sketch`'s plane and body holding its geometry and
/// `with`'s, each mapped onto the plane.
pub(crate) fn merge(
    document: &mut core_document::Document,
    sketch: FeatureId,
    with: &[FeatureId],
) -> Result<FeatureId, CommandError> {
    let base =
        placed(document, sketch).ok_or_else(|| CommandError::bad("sketch", "is not a sketch"))?;
    let mut merged = Sketch::new(format!("{} merged", base.sketch.name));
    crate::copy_sketch_into(
        &base.sketch,
        &mut merged,
        &crate::tools::Similarity::translation(glam::DVec2::ZERO),
    );
    for other in with {
        let from = placed(document, *other)
            .ok_or_else(|| CommandError::bad("with", "holds something that is not a sketch"))?;
        let xf = crate::plane_map(&from.plane, &base.plane).map_err(CommandError::failed)?;
        crate::copy_sketch_into(&from.sketch, &mut merged, &xf);
    }
    new_beside(document, sketch, merged)
}

/// A new sketch on `sketch`'s plane and body: its geometry mirrored across
/// the sketch's Y axis.
pub(crate) fn mirror_sketch(
    document: &mut core_document::Document,
    sketch: FeatureId,
) -> Result<FeatureId, CommandError> {
    let base = crate::stored_sketch(document, sketch)
        .ok_or_else(|| CommandError::bad("sketch", "is not a sketch"))?;
    let mut mirrored = Sketch::new(format!("{} mirror", base.sketch.name));
    let all: std::collections::HashSet<Uuid> = base
        .sketch
        .geometry
        .iter()
        .map(GeometryElement::id)
        .collect();
    let xf = crate::tools::Similarity::mirror_about(glam::DVec2::ZERO, glam::DVec2::Y);
    crate::tools::copy_from(&base.sketch, &mut mirrored, &all, &xf);
    new_beside(document, sketch, mirrored)
}

/// Add `made` as a new sketch on `beside`'s stored plane and in its body.
fn new_beside(
    document: &mut core_document::Document,
    beside: FeatureId,
    mut made: Sketch,
) -> Result<FeatureId, CommandError> {
    let stored = crate::stored_sketch(document, beside)
        .ok_or_else(|| CommandError::bad("sketch", "is not a sketch"))?;
    let body = document.get_feature_meta(beside).and_then(|n| n.body);
    made.plane = stored.plane;
    let name = made.name.clone();
    document
        .add_feature_in_body(SketchFeature::new(made, stored.plane), name, body)
        .map_err(|e| CommandError::failed(e.to_string()))
}

/// `sources` counting in the profile or only guiding, as the call's
/// `counts` says (guiding when it says nothing).
fn counting(
    a: &Args,
    mut sources: Vec<crate::sketch::ExternalSource>,
) -> Result<Vec<crate::sketch::ExternalSource>, CommandError> {
    let counts = a.opt_bool("counts")?.unwrap_or(false);
    for source in &mut sources {
        source.defining = counts;
    }
    Ok(sources)
}

/// Edges named as `{body, point, direction}`, in each body's own frame.
fn external_sources(
    value: Option<&Value>,
) -> Result<Vec<crate::sketch::ExternalSource>, CommandError> {
    let bad = || CommandError::bad("edges", "must be a list of {body, point, direction}");
    let list = value.and_then(Value::as_array).ok_or_else(bad)?;
    list.iter()
        .map(|edge| {
            let body = edge
                .get("body")
                .and_then(Value::as_str)
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(bad)?;
            let v = |name: &str| -> Result<[f64; 3], CommandError> {
                let v = vector3(edge.get(name), "edges")?;
                Ok(v.map(|c| c))
            };
            Ok(crate::sketch::ExternalSource {
                body,
                point: v("point")?,
                direction: v("direction")?,
                section: false,
                defining: false,
                reference: None,
                faces: [0; 2],
            })
        })
        .collect()
}

/// Faces named as `{body, point, normal}`, in each body's own frame.
fn section_sources(
    value: Option<&Value>,
) -> Result<Vec<crate::sketch::ExternalSource>, CommandError> {
    let bad = || CommandError::bad("faces", "must be a list of {body, point, normal}");
    let list = value.and_then(Value::as_array).ok_or_else(bad)?;
    list.iter()
        .map(|face| {
            let body = face
                .get("body")
                .and_then(Value::as_str)
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(bad)?;
            let v = |name: &str| -> Result<[f64; 3], CommandError> {
                let v = vector3(face.get(name), "faces")?;
                Ok(v.map(|c| c))
            };
            Ok(crate::sketch::ExternalSource {
                body,
                point: v("point")?,
                direction: v("normal")?,
                section: true,
                defining: false,
                reference: None,
                faces: [0; 2],
            })
        })
        .collect()
}

/// Bring `edges` (edges to project, faces to cut) onto `plane` (the
/// sketch's, where its body sits) and add what they come to as external
/// geometry. How many elements came.
pub(crate) fn add_external(
    ctx: &WorkbenchRuntimeContext,
    plane: &SketchPlane,
    sketch: &mut Sketch,
    edges: &[crate::sketch::ExternalSource],
) -> Result<usize, String> {
    let mut added = 0;
    let mut repeated = 0;
    let mut last_error = None;
    for source in edges {
        let source = &crate::with_edge_names(ctx.document, *source);
        match crate::project_source(ctx, plane, source) {
            Ok(curves) => {
                // An edge or element the sketch already holds comes once.
                let held = crate::external::groups(sketch)
                    .iter()
                    .any(|(_, group)| crate::external::already_holds(sketch, group, &curves));
                if held {
                    repeated += 1;
                    continue;
                }
                for curve in &curves {
                    added += crate::external::add(sketch, curve, *source);
                }
            }
            Err(why) => last_error = Some(why),
        }
    }
    match (added, last_error) {
        (0, Some(why)) => Err(why),
        (0, None) if repeated > 0 => Err(match repeated {
            1 => "that is already in the sketch as external geometry".to_string(),
            _ => "those are already in the sketch as external geometry".to_string(),
        }),
        _ => Ok(added),
    }
}

/// What `from` brings as external geometry: each curve and loose point of a
/// sketch (its construction left out), or a datum; `None` for anything
/// else.
pub(crate) fn reference_sources(
    document: &core_document::Document,
    from: FeatureId,
) -> Option<Vec<crate::sketch::ExternalSource>> {
    use crate::sketch::{ExternalReference, ExternalSource};
    let node = document.get_feature_meta(from)?;
    if node.workbench_id.as_str() == core_document::DATUM_KIND {
        return Some(vec![ExternalSource::of_reference(
            ExternalReference::Datum { datum: from.0 },
        )]);
    }
    let other = crate::stored_sketch(document, from)?;
    let sketch = &other.sketch;
    let on_curves: std::collections::HashSet<Uuid> = sketch
        .geometry
        .iter()
        .flat_map(Sketch::curve_point_ids)
        .collect();
    Some(
        sketch
            .geometry
            .iter()
            .filter(|g| !sketch.is_construction(g.id()) && !sketch.is_external(g.id()))
            .filter(|g| !matches!(g, GeometryElement::Point(p) if on_curves.contains(&p.id)))
            .map(|g| {
                ExternalSource::of_reference(ExternalReference::SketchElement {
                    sketch: from.0,
                    element: g.id(),
                })
            })
            .collect(),
    )
}

/// Mark `items` of the sketch's external geometry as counting in its
/// profiles, or not.
pub(crate) fn set_external_defining(sketch: &mut Sketch, items: &[Uuid], on: bool) {
    for id in items {
        if let Some(source) = sketch.external.get_mut(id) {
            source.defining = on;
            // Counting in the profile is what makes it normal geometry.
            if on {
                sketch.construction.remove(id);
            }
        }
    }
}

/// Named arguments from a JSON object.
pub(crate) fn args(value: Value) -> CommandArgs {
    match value {
        Value::Object(map) => map,
        _ => CommandArgs::new(),
    }
}

/// Delete elements and constraints, and what hangs on them. The origin and
/// the axes are not the sketch's to delete. How many went.
pub(crate) fn delete_items(sketch: &mut Sketch, items: &[Uuid]) -> usize {
    let (constraints, elements): (Vec<Uuid>, Vec<Uuid>) = items
        .iter()
        .copied()
        .partition(|id| sketch.constraints.iter().any(|c| c.id == *id));
    let before = sketch.constraints.len();
    sketch.constraints.retain(|c| !constraints.contains(&c.id));
    let elements: Vec<Uuid> = elements
        .into_iter()
        .filter(|id| crate::sketch::Reference::of(*id).is_none())
        .collect();
    before - sketch.constraints.len() + sketch.remove_geometry_cascade(&elements).len()
}

/// `sketch.draw`: the tool run over the points as clicks, the same step a
/// click in the viewport takes. The ids of what it made.
fn draw(
    sketch: &mut Sketch,
    a: &Args,
    args: &CommandArgs,
) -> Result<(Vec<String>, Vec<String>), CommandError> {
    use crate::step;
    let name = a.string("tool")?;
    let tool = if name.starts_with("sketch.") {
        name.to_string()
    } else {
        format!("sketch.{name}")
    };
    if !DRAW_TOOLS.contains(&tool.trim_start_matches("sketch.")) {
        return Err(CommandError::bad("tool", format!("has no tool `{name}`")));
    }
    let settings = step::StepSettings {
        tol: a.opt_number("tolerance")?.unwrap_or(1e-3),
        params: step::params_from_json(args.get("params"))
            .map_err(|e| CommandError::bad("params", e))?,
        construction: a.opt_bool("construction")?.unwrap_or(false),
        avoid_redundant: a.opt_bool("avoid_redundant")?.unwrap_or(true),
    };
    let selected: std::collections::HashSet<Uuid> = match args.get("selection") {
        Some(v) if !v.is_null() => ids(Some(v), "selection", sketch)?.into_iter().collect(),
        _ => Default::default(),
    };
    let events = args
        .get("points")
        .and_then(Value::as_array)
        .ok_or_else(|| CommandError::bad("points", "must be a list of clicks"))?;
    let elements_before: std::collections::HashSet<Uuid> =
        sketch.geometry.iter().map(GeometryElement::id).collect();
    let constraints_before: std::collections::HashSet<Uuid> =
        sketch.constraints.iter().map(|c| c.id).collect();
    let mut state = crate::tools::ToolState::Idle;
    let mut capture = crate::ovp::DimCapture::default();
    for event in events {
        match event {
            Value::String(word) => match word.as_str() {
                w if crate::tools::PolySegment::of_word(w).is_some() => {
                    if let Some(segment) = crate::tools::PolySegment::of_word(w) {
                        crate::tools::set_polyline_segment(&mut state, segment);
                    }
                }
                "finish" => {
                    let effect =
                        crate::tools::finish_click_sequence(&mut state, sketch, &settings.params);
                    if effect.changed {
                        crate::solver::solve(sketch);
                    }
                }
                other => {
                    return Err(CommandError::bad(
                        "points",
                        format!(
                            "has `{other}`; a word there is line, arc, perpendicular_arc, \
                             reverse_arc or finish"
                        ),
                    ));
                }
            },
            // A line tool's arc: tangent to what ends where it draws from.
            click
                if tool == "sketch.line"
                    && click.get("arc").and_then(Value::as_bool) == Some(true) =>
            {
                let (at, typed, constrain) = click_of(click)?;
                let outcome = step::line_arc_click(
                    &mut state,
                    &mut capture,
                    sketch,
                    at,
                    &typed,
                    constrain,
                    &settings,
                    &selected,
                );
                if outcome.changed {
                    crate::solver::solve(sketch);
                }
            }
            click => {
                let (at, typed, constrain) = click_of(click)?;
                let outcome = step::click(
                    &mut state,
                    &mut capture,
                    &tool,
                    sketch,
                    at,
                    &typed,
                    constrain,
                    &settings,
                    &selected,
                );
                // The sketch settles after every click that changes it, as
                // it does between clicks in the viewport, so the next click
                // meets the same sketch.
                if outcome.changed || outcome.added > 0 {
                    crate::solver::solve(sketch);
                }
            }
        }
    }
    let elements = sketch
        .geometry
        .iter()
        .map(GeometryElement::id)
        .filter(|id| !elements_before.contains(id))
        .map(|id| id.to_string())
        .collect();
    let constraints = sketch
        .constraints
        .iter()
        .map(|c| c.id)
        .filter(|id| !constraints_before.contains(id))
        .map(|id| id.to_string())
        .collect();
    Ok((elements, constraints))
}

/// The tools `sketch.draw` runs, without the `sketch.` prefix.
const DRAW_TOOLS: &[&str] = &[
    "point",
    "line",
    "polyline",
    "rect",
    "rect_rounded",
    "rect_center",
    "rect3",
    "rect_center3",
    "rect_frame",
    "circle",
    "circle3",
    "arc",
    "arc3",
    "ellipse",
    "ellipse3",
    "ellipse_arc",
    "parabola",
    "hyperbola",
    "bspline",
    "polygon",
    "slot",
    "arc_slot",
    "fillet",
    "chamfer",
    "trim",
    "extend",
    "split",
    "bspline_knot",
    "offset",
    "translate",
    "rotate",
    "scale",
    "mirror",
];

/// One click of `sketch.draw`: where, the values typed at it, and whether
/// they become constraints.
type Click = (Vec2D, Vec<(crate::ovp::FieldKind, f64)>, bool);

/// Read one click of `sketch.draw`.
fn click_of(click: &Value) -> Result<Click, CommandError> {
    let at = points(Some(&json!([click])))
        .map_err(|_| CommandError::bad("points", "must hold {x, y} clicks"))?[0];
    let mut typed = Vec::new();
    let mut constrain = false;
    if let Value::Object(fields) = click {
        if let Some(Value::Object(values)) = fields.get("typed") {
            for (name, v) in values {
                let kind = crate::step::field_of(name).ok_or_else(|| {
                    CommandError::bad("points", format!("types `{name}`, which no tool asks for"))
                })?;
                let v = v
                    .as_f64()
                    .ok_or_else(|| CommandError::bad("points", "typed values are numbers"))?;
                typed.push((kind, v));
            }
        }
        constrain = fields
            .get("constrain")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    }
    Ok((at, typed, constrain))
}

/// Add the constraints `kind` makes for `items`, at `value` when given.
pub(crate) fn constrain(
    sketch: &mut Sketch,
    kind: &str,
    items: &[Uuid],
    value: Option<f64>,
) -> Result<Vec<String>, CommandError> {
    if kind != "dimension" && !crate::constrain::TOOLS.contains(&kind) {
        return Err(CommandError::bad(
            "kind",
            format!(
                "has `{kind}`, which is no constraint; the kinds are dimension, {}",
                crate::constrain::TOOLS.join(", ")
            ),
        ));
    }
    let selected: std::collections::HashSet<Uuid> = items.iter().copied().collect();
    let shape = crate::constrain::SelectionShape::picked_in(sketch, &selected, items);
    let tool = if kind == "dimension" {
        crate::constrain::dimension_in(&shape, sketch)
            .ok_or_else(|| CommandError::failed("dimension takes a line, circles, or two items"))?
    } else {
        kind
    };
    let kinds = crate::constrain::kinds_for(tool, &shape, sketch).ok_or_else(|| {
        CommandError::failed(format!("the {tool} constraint does not fit these items"))
    })?;
    // A value goes to the dimension among them; the constraints that come
    // with it (a point kept on a curve, the equal radii) take none.
    if value.is_some()
        && !kinds
            .iter()
            .any(|k| crate::sketch::dimension_value(k).is_some())
    {
        return Err(CommandError::bad("value", format!("{tool} takes no value")));
    }
    let mut made = Vec::new();
    for mut k in kinds {
        if let Some(value) = value
            && crate::sketch::dimension_value(&k).is_some()
        {
            k = crate::sketch::with_dimension_value(&k, value);
        }
        made.push(sketch.add_constraint(k).to_string());
    }
    Ok(made)
}

/// Take away the older constraints the constraints `made` (their ids, as
/// [`constrain`] returns them) made redundant; returns how many went.
pub(crate) fn remove_superseded(sketch: &mut Sketch, made: &[String]) -> usize {
    let new: Vec<Uuid> = made.iter().filter_map(|id| id.parse().ok()).collect();
    let gone = crate::solver::superseded(sketch, &new);
    sketch.constraints.retain(|c| !gone.contains(&c.id));
    gone.len()
}

/// The name of a constraint kind, as it is stored.
fn kind_name(kind: &crate::sketch::ConstraintKind) -> String {
    match serde_json::to_value(kind) {
        Ok(Value::Object(m)) => m.keys().next().cloned().unwrap_or_default(),
        Ok(Value::String(s)) => s,
        _ => String::new(),
    }
}

fn describe_constraints(sketch: &Sketch) -> Value {
    Value::Array(
        sketch
            .constraints
            .iter()
            .map(|c| {
                let mut out = json!({
                    "id": c.id.to_string(),
                    "kind": kind_name(&c.kind),
                    "items": crate::sketch::constraint_refs(&c.kind).iter().map(Uuid::to_string).collect::<Vec<_>>(),
                });
                if let Some(v) = crate::sketch::dimension_value(&c.kind) {
                    out["value"] = json!(v);
                }
                out
            })
            .collect(),
    )
}

/// The regions the sketch's curves close, each `{name, area, x, y,
/// default}` with a point inside it.
fn describe_regions(sketch: &Sketch) -> Result<Value, CommandError> {
    let regions =
        crate::profile::regions(sketch).map_err(|e| CommandError::failed(e.to_string()))?;
    Ok(Value::Array(
        regions
            .iter()
            .map(|r| {
                json!({
                    "name": r.name,
                    "area": r.area,
                    "x": r.inside[0],
                    "y": r.inside[1],
                    "default": r.default,
                })
            })
            .collect(),
    ))
}

fn status(sketch: &Sketch) -> Value {
    let mut solved = sketch.clone();
    let outcome = crate::solver::solve(&mut solved);
    let diagnosis = crate::solver::diagnose(sketch);
    let list = |ids: &[Uuid]| ids.iter().map(Uuid::to_string).collect::<Vec<_>>();
    json!({
        "dof": diagnosis.dof,
        "solved": !matches!(outcome, crate::solver::SolveOutcome::NotConverged { .. }),
        "redundant": list(&diagnosis.redundant),
        "conflicting": list(&diagnosis.conflicting),
    })
}

/// A list of `{x, y}` pairs, as `{{0, 0}, {10, 0}}` or `{{x = 0, y = 0}}`.
fn points(value: Option<&Value>) -> Result<Vec<Vec2D>, CommandError> {
    let bad = || CommandError::bad("points", "must be a list of {x, y} pairs");
    let list = value.and_then(Value::as_array).ok_or_else(bad)?;
    list.iter()
        .map(|p| {
            let (x, y) = match p {
                Value::Array(xy) if xy.len() == 2 => (xy[0].as_f64(), xy[1].as_f64()),
                Value::Object(xy) => (
                    xy.get("x").and_then(Value::as_f64),
                    xy.get("y").and_then(Value::as_f64),
                ),
                _ => (None, None),
            };
            Ok(vec(x.ok_or_else(bad)?, y.ok_or_else(bad)?))
        })
        .collect()
}

fn describe(sketch: &Sketch) -> Value {
    let at = |id: Uuid| {
        sketch
            .point_position(id)
            .map(|p| json!([p.x, p.y]))
            .unwrap_or(Value::Null)
    };
    let elements: Vec<Value> = sketch
        .geometry
        .iter()
        .map(|g| {
            let (kind, points, radius) = match g {
                GeometryElement::Point(p) => ("point", vec![at(p.id)], None),
                GeometryElement::Line(l) => ("line", vec![at(l.start), at(l.end)], None),
                GeometryElement::Circle(c) => ("circle", vec![at(c.center)], Some(c.radius)),
                GeometryElement::Arc(a) => (
                    "arc",
                    vec![at(a.center), at(a.start), at(a.end)],
                    Some(a.radius),
                ),
                GeometryElement::Ellipse(e) => ("ellipse", vec![at(e.center)], None),
                _ => ("other", Vec::new(), None),
            };
            let mut out = json!({
                "id": g.id().to_string(),
                "kind": kind,
                "points": points,
                "construction": sketch.construction.contains(&g.id()),
                "external": sketch.external.contains_key(&g.id()),
            });
            if let Some(r) = radius {
                out["radius"] = json!(r);
            }
            out
        })
        .collect();
    Value::Array(elements)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::{Document, WorkbenchRuntimeContext};

    fn call(doc: &mut Document, id: &str, args: Value) -> CommandResult {
        let mut ctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
        run(id, args.as_object().unwrap(), &mut ctx)
    }

    /// A call that must succeed: its answer.
    /// Whether the call is refused.
    fn refused(doc: &mut Document, id: &str, args: Value) -> bool {
        call(doc, id, args).is_err()
    }

    fn ok(doc: &mut Document, id: &str, args: Value) -> Value {
        call(doc, id, args).unwrap_or_else(|e| panic!("{id}: {e}"))
    }

    fn feature_id(id: &Value) -> FeatureId {
        FeatureId(Uuid::parse_str(id.as_str().unwrap()).unwrap())
    }

    /// The stored sketch feature a command answered with the id of.
    fn feature_of(doc: &Document, id: &Value) -> SketchFeature {
        SketchFeature::from_json(doc.get_feature_data(feature_id(id)).unwrap()).unwrap()
    }

    fn sketch_of(doc: &Document, id: &Value) -> Sketch {
        feature_of(doc, id).sketch
    }

    /// A picture file comes in as a document asset and lies in a new
    /// sketch named after it; it moves, sizes and goes by its id.
    #[test]
    fn a_reference_picture_comes_in_and_is_placed() {
        let path = std::env::temp_dir().join(format!("printcad-ref-{}.png", Uuid::new_v4()));
        image::RgbaImage::from_pixel(4, 2, image::Rgba([200, 100, 50, 255]))
            .save(&path)
            .unwrap();
        let mut doc = Document::new("t");
        let made = ok(
            &mut doc,
            "sketch.image",
            json!({"path": path.to_string_lossy(), "width": 40.0}),
        );
        let _ = std::fs::remove_file(&path);
        let sketch = FeatureId(Uuid::parse_str(made["sketch"].as_str().unwrap()).unwrap());
        let image = made["image"].as_str().unwrap().to_string();
        let feature = SketchFeature::from_json(doc.get_feature_data(sketch).unwrap()).unwrap();
        assert!(
            doc.get_feature_meta(sketch)
                .unwrap()
                .name
                .starts_with("printcad-ref-")
        );
        let placed = &feature.sketch.images[0];
        assert_eq!(placed.width, 40.0);
        assert!(doc.asset_bytes(placed.asset).is_some(), "the file is kept");

        ok(
            &mut doc,
            "sketch.set_image",
            json!({"sketch": sketch.0.to_string(), "image": image, "x": 5.0, "angle": 30.0}),
        );
        let feature = SketchFeature::from_json(doc.get_feature_data(sketch).unwrap()).unwrap();
        let moved = &feature.sketch.images[0];
        assert_eq!(
            (moved.center.x, moved.angle_deg, moved.width),
            (5.0, 30.0, 40.0)
        );

        ok(
            &mut doc,
            "sketch.set_image",
            json!({"sketch": sketch.0.to_string(), "image": image, "remove": true}),
        );
        let feature = SketchFeature::from_json(doc.get_feature_data(sketch).unwrap()).unwrap();
        assert!(feature.sketch.images.is_empty());
        assert!(refused(
            &mut doc,
            "sketch.image",
            json!({"path": "/nonexistent/picture.png"})
        ));
    }

    /// `sketch.new{on = datum}` draws the sketch on the datum and keeps it
    /// there: the sketch records the datum and depends on it; a plane set
    /// outright later lets it go.
    #[test]
    fn a_sketch_made_on_a_datum_keeps_to_it() {
        use core_document::{
            AttachmentOffset, BasePlane, DatumAttachment, DatumFeature, DatumShape,
        };
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let datum = doc
            .add_feature_in_body(
                DatumFeature {
                    shape: DatumShape::CoordinateSystem { size: 10.0 },
                    attachment: DatumAttachment::BasePlane(BasePlane::XY),
                    offset: AttachmentOffset {
                        tilt: [0.0; 2],
                        translation: [0.0, 0.0, 5.0],
                        rotation_deg: 0.0,
                        flip: false,
                    },
                },
                "Frame".into(),
                Some(body),
            )
            .unwrap();
        let made = ok(
            &mut doc,
            "sketch.new",
            json!({"on": datum.0.to_string(), "plane": "xz", "offset": 2.0}),
        );
        let id = feature_id(&made);
        let feature = feature_of(&doc, &made);
        assert_eq!(
            feature.support,
            Some(DatumSupport {
                datum,
                plane: Some("XZ".into()),
                offset: 2.0,
                shift: [0.0, 0.0],
                turn: 0.0,
            })
        );
        assert_eq!(doc.feature_tree().dependencies(id), vec![datum]);
        assert_eq!(doc.get_feature_meta(id).unwrap().body, Some(body));

        ok(
            &mut doc,
            "sketch.set_plane",
            json!({"sketch": made, "normal": [0, 0, 1]}),
        );
        let feature = SketchFeature::from_json(doc.get_feature_data(id).unwrap()).unwrap();
        assert_eq!(feature.support, None);
        assert!(doc.feature_tree().dependencies(id).is_empty());
    }

    #[test]
    fn a_new_sketch_can_be_a_generators() {
        let mut doc = Document::new("t");
        let made = ok(
            &mut doc,
            "sketch.new",
            json!({"plane": "XZ", "generator": "sprocket"}),
        );
        let id = feature_id(&made);
        let feature = feature_of(&doc, &made);
        assert!(matches!(
            feature.generator,
            Some(crate::generator::Generator::Sprocket(_))
        ));
        assert!(!feature.sketch.geometry.is_empty());
        assert_eq!(feature.plane.normal, SketchPlane::xz().normal);
        assert_eq!(doc.get_feature_meta(id).unwrap().name, "Sprocket");
        let again = ok(&mut doc, "sketch.new", json!({"generator": "sprocket"}));
        let again = feature_id(&again);
        assert_eq!(doc.get_feature_meta(again).unwrap().name, "Sprocket_1");
        assert!(refused(&mut doc, "sketch.new", json!({"generator": "cam"})));
    }

    #[test]
    fn scripted_text_is_laid_out_and_changed() {
        let mut doc = Document::new("t");
        let sketch = ok(&mut doc, "sketch.new", json!({"plane": "XY"}));
        let made = ok(
            &mut doc,
            "sketch.text",
            json!({"sketch": sketch, "text": "Hi", "at": [2, 3], "size": 8}),
        );
        let id = feature_id(&sketch);
        let read = |doc: &Document| {
            SketchFeature::from_json(doc.get_feature_data(id).unwrap())
                .unwrap()
                .sketch
        };
        let s = read(&doc);
        assert_eq!(s.texts.len(), 1);
        assert_eq!(made["point"], json!(s.texts[0].anchor.to_string()));
        let wires = crate::profile::extract_wires(&s).unwrap().len();
        assert!(wires >= 3, "an H and an i: {wires}");
        ok(
            &mut doc,
            "sketch.text_edit",
            json!({"sketch": sketch, "block": made["text"], "text": "HHH"}),
        );
        let s = read(&doc);
        assert_eq!(s.texts[0].text, "HHH");
        assert_eq!(crate::profile::extract_wires(&s).unwrap().len(), 3);
        assert!(refused(
            &mut doc,
            "sketch.text",
            json!({"sketch": sketch, "text": "x", "at": [0, 0], "font": "/no/such.ttf"})
        ));
    }

    #[test]
    fn a_scripted_rectangle_is_one_closed_profile() {
        let mut doc = Document::new("t");
        let sketch = ok(&mut doc, "sketch.new", json!({"plane": "XZ"}));
        let lines = ok(
            &mut doc,
            "sketch.rect",
            json!({"sketch": sketch, "x": 0, "y": 0, "width": 20, "height": 10}),
        );
        assert_eq!(lines.as_array().unwrap().len(), 4);
        let id = feature_id(&sketch);
        let feature = feature_of(&doc, &sketch);
        let points = feature
            .sketch
            .geometry
            .iter()
            .filter(|g| matches!(g, GeometryElement::Point(_)))
            .count();
        assert_eq!(points, 4, "corners are shared");
        assert_eq!(
            crate::profile::extract_wires(&feature.sketch)
                .unwrap()
                .len(),
            1
        );
        assert!(doc.get_feature_meta(id).unwrap().body.is_some());
    }

    /// A negative horizontal or vertical distance puts the end that far
    /// the other way; the line keeps its length and nothing conflicts.
    #[test]
    fn a_negative_axis_distance_runs_the_other_way() {
        for (kind, axis) in [("distance_x", 0), ("distance_y", 1)] {
            let mut doc = Document::new("t");
            let sketch = ok(&mut doc, "sketch.new", json!({}));
            let line = ok(
                &mut doc,
                "sketch.line",
                json!({"sketch": sketch, "x1": 0, "y1": 0, "x2": 5, "y2": 3}),
            );
            ok(
                &mut doc,
                "sketch.constrain",
                json!({"sketch": sketch, "kind": kind, "items": [line], "value": -7}),
            );
            let status = ok(&mut doc, "sketch.status", json!({"sketch": sketch}));
            assert_eq!(status["conflicting"], json!([]), "{kind}");
            let geometry = ok(&mut doc, "sketch.geometry", json!({"sketch": sketch}));
            let points = geometry
                .as_array()
                .unwrap()
                .iter()
                .find(|g| g["kind"] == "line")
                .unwrap()["points"]
                .clone();
            let d = points[1][axis].as_f64().unwrap() - points[0][axis].as_f64().unwrap();
            assert!((d + 7.0).abs() < 1e-3, "{kind}: {points}");
        }
    }

    /// What is left free: 5 for an arc, however drawn, and for an ellipse.
    #[test]
    fn arcs_and_ellipses_count_five_degrees_of_freedom() {
        let dof = |tool: &str, points: Value| {
            let mut doc = Document::new("t");
            let sketch = ok(&mut doc, "sketch.new", json!({}));
            ok(
                &mut doc,
                "sketch.draw",
                json!({"sketch": sketch, "tool": tool, "points": points}),
            );
            ok(&mut doc, "sketch.status", json!({"sketch": sketch}))["dof"].clone()
        };
        assert_eq!(dof("arc3", json!([[0, 0], [10, 0], [5, 3]])), json!(5));
        assert_eq!(dof("ellipse", json!([[0, 0], [10, 0], [0, 4]])), json!(5));
    }

    /// A rounded rectangle, a slot and an arc slot keep their shape when
    /// dragged: their tangents and equal radii are part of them, and none
    /// of those constraints says again what the others already do.
    #[test]
    fn shapes_with_rounded_ends_hold_together_when_dragged() {
        for (tool, points, params, dof) in [
            (
                "rect_rounded",
                json!([[0, 0], [20, 10]]),
                json!({"fillet_radius": 2}),
                5,
            ),
            (
                "slot",
                json!([[1, 1], [21, 1]]),
                json!({"slot_width": 6}),
                5,
            ),
            (
                "arc_slot",
                json!([[0, 0], [20, 0], [0, 20]]),
                json!({"slot_width": 4}),
                6,
            ),
        ] {
            let mut doc = Document::new("t");
            let sketch = ok(&mut doc, "sketch.new", json!({}));
            ok(
                &mut doc,
                "sketch.draw",
                json!({"sketch": sketch, "tool": tool, "points": points, "params": params}),
            );
            let status = ok(&mut doc, "sketch.status", json!({"sketch": sketch}));
            assert_eq!(status["dof"], json!(dof), "{tool}");
            assert_eq!(status["redundant"], json!([]), "{tool}");
            let geometry = ok(&mut doc, "sketch.geometry", json!({"sketch": sketch}));
            let first = geometry
                .as_array()
                .unwrap()
                .iter()
                .find(|g| g["kind"] == "line" || g["kind"] == "arc")
                .unwrap()["id"]
                .clone();
            ok(
                &mut doc,
                "sketch.drag",
                json!({"sketch": sketch, "items": [first], "by": [1.5, 2.5]}),
            );
            let geometry = ok(&mut doc, "sketch.geometry", json!({"sketch": sketch}));
            let radii: Vec<f64> = geometry
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|g| g["radius"].as_f64())
                .collect();
            // The ends (all four corners, both caps) stay one size.
            let ends: Vec<f64> = match tool {
                "arc_slot" => radii[2..].to_vec(),
                _ => radii.clone(),
            };
            assert!(
                ends.windows(2).all(|w| (w[0] - w[1]).abs() < 1e-3),
                "{tool}: {radii:?}"
            );
        }
    }

    /// Rows and columns are the array's, not a drawing tool's: sketch.draw
    /// says so rather than ignoring them.
    #[test]
    fn array_settings_given_to_a_drawing_tool_are_refused() {
        let mut doc = Document::new("t");
        let sketch = ok(&mut doc, "sketch.new", json!({}));
        let error = call(
            &mut doc,
            "sketch.draw",
            json!({"sketch": sketch, "tool": "translate", "points": [[0, 0], [5, 0]],
                   "params": {"array_rows": 3}}),
        )
        .unwrap_err();
        assert!(error.to_string().contains("sketch.array"), "{error}");
    }

    /// A rectangle keeps its sides level and upright, as the drawing
    /// tool's does, and so do a polyline's level and upright segments.
    #[test]
    fn a_rectangle_and_a_polyline_hold_their_level_and_upright_sides() {
        let mut doc = Document::new("t");
        let sketch = ok(&mut doc, "sketch.new", json!({}));
        ok(
            &mut doc,
            "sketch.rect",
            json!({"sketch": sketch, "x": 0, "y": 0, "width": 10, "height": 5}),
        );
        ok(
            &mut doc,
            "sketch.polyline",
            json!({"sketch": sketch, "points": [[20, 0], [30, 0], [34, 6]]}),
        );
        let kinds: Vec<String> = ok(&mut doc, "sketch.constraints", json!({"sketch": sketch}))
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["kind"].as_str().unwrap().to_string())
            .collect();
        let count = |k: &str| kinds.iter().filter(|x| *x == k).count();
        assert_eq!(
            (count("Horizontal"), count("Vertical")),
            (3, 2),
            "{kinds:?}"
        );
    }

    #[test]
    fn lines_drawn_end_to_end_share_their_points() {
        let mut doc = Document::new("t");
        let sketch = ok(&mut doc, "sketch.new", json!({}));
        for (x1, y1, x2, y2) in [(0, 0, 10, 0), (10, 0, 0, 10), (0, 10, 0, 0)] {
            ok(
                &mut doc,
                "sketch.line",
                json!({"sketch": sketch, "x1": x1, "y1": y1, "x2": x2, "y2": y2}),
            );
        }
        let listed = ok(&mut doc, "sketch.geometry", json!({"sketch": sketch}));
        let points = listed
            .as_array()
            .unwrap()
            .iter()
            .filter(|g| g["kind"] == "point")
            .count();
        assert_eq!(points, 3);
    }

    /// A face given as an attachment is on the sketch's own body: the
    /// sketch follows it unless told not to.
    #[test]
    fn an_attached_sketch_follows_its_face_unless_told_not_to() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        for (given, follows) in [(None, true), (Some(false), false)] {
            let mut face = json!({"point": [0, 0, 10], "normal": [0, 0, 1]});
            if let Some(given) = given {
                face["follows"] = json!(given);
            }
            let sketch = ok(
                &mut doc,
                "sketch.new",
                json!({"body": body.0.to_string(), "attachment": {"Face": {"face": face}}}),
            );
            let feature = feature_of(&doc, &sketch);
            let Some(core_document::DatumAttachment::Face { face }) =
                feature.attached.map(|a| a.attachment)
            else {
                panic!("attached on the face");
            };
            assert_eq!(face.follows, follows);
        }
    }

    #[test]
    fn a_sketch_attached_by_three_points_lies_on_them() {
        use core_document::{DatumAttachment, PointAnchor};
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let at = |point| PointAnchor::At { point };
        let attachment = DatumAttachment::ThreePoints {
            points: [
                at([0.0, 0.0, 5.0]),
                at([10.0, 0.0, 5.0]),
                at([0.0, 10.0, 5.0]),
            ],
        };
        let sketch = ok(
            &mut doc,
            "sketch.new",
            json!({"body": body.0.to_string(), "attachment": attachment}),
        );
        let feature = feature_of(&doc, &sketch);
        assert_eq!(feature.attached.map(|a| a.attachment), Some(attachment));
        let (o, n) = (feature.plane.origin, feature.plane.normal);
        assert!((o[2] - 5.0).abs() < 1e-5, "{o:?}");
        assert!((n[2].abs() - 1.0).abs() < 1e-5, "{n:?}");
        assert!(refused(
            &mut doc,
            "sketch.new",
            json!({"body": body.0.to_string(), "attachment": {"Nonsense": 1}})
        ));
    }

    #[test]
    fn a_sketch_on_a_datum_shifts_and_turns_on_it() {
        use core_document::{
            AttachmentOffset, BasePlane, DatumAttachment, DatumFeature, DatumShape,
        };
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let datum = doc
            .add_feature_in_body(
                DatumFeature {
                    shape: DatumShape::Plane { size: 20.0 },
                    attachment: DatumAttachment::BasePlane(BasePlane::XY),
                    offset: AttachmentOffset::default(),
                },
                "Datum".into(),
                Some(body),
            )
            .unwrap();
        let sketch = ok(&mut doc, "sketch.new", json!({"on": datum.0.to_string()}));
        ok(
            &mut doc,
            "sketch.attachment",
            json!({"sketch": sketch, "offset": 3.0, "shift": [5.0, 0.0], "turn": 90.0}),
        );
        let feature = feature_of(&doc, &sketch);
        let near = |a: [f64; 3], b: [f64; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5);
        assert!(
            near(feature.plane.origin, [5.0, 0.0, 3.0]),
            "{:?}",
            feature.plane
        );
        assert!(
            near(feature.plane.x_axis, [0.0, 1.0, 0.0]),
            "{:?}",
            feature.plane
        );
        assert!(near(feature.plane.y_axis, [-1.0, 0.0, 0.0]));
        let free = ok(&mut doc, "sketch.new", json!({}));
        assert!(
            refused(
                &mut doc,
                "sketch.attachment",
                json!({"sketch": free, "turn": 5.0})
            ),
            "only a sketch on a datum"
        );
    }

    #[test]
    fn another_sketch_and_a_datum_come_in_as_external_geometry_that_follows_them() {
        use core_document::{
            AttachmentOffset, BasePlane, DatumAttachment, DatumFeature, DatumShape,
        };
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let first = ok(&mut doc, "sketch.new", json!({"body": body.0.to_string()}));
        for (x1, y1, x2, y2) in [(0, 0, 10, 0), (10, 0, 10, 6), (10, 6, 0, 6), (0, 6, 0, 0)] {
            ok(
                &mut doc,
                "sketch.line",
                json!({"sketch": first, "x1": x1, "y1": y1, "x2": x2, "y2": y2}),
            );
        }
        let second = ok(&mut doc, "sketch.new", json!({"body": body.0.to_string()}));
        let made = ok(
            &mut doc,
            "sketch.external_from",
            json!({"sketch": second, "from": first}),
        );
        assert_eq!(
            made["elements"].as_array().map(Vec::len),
            Some(12),
            "four lines and their points: {made}"
        );
        let copied = sketch_of(&doc, &second);
        assert_eq!(
            copied.external.len(),
            4,
            "the lines; their points go with them"
        );
        assert!(copied.external.values().all(|s| s.reference.is_some()));
        assert!(
            copied.external.values().all(|s| !s.defining),
            "said nothing, it only guides"
        );
        // Asked to count, it closes a profile of its own.
        let third = ok(&mut doc, "sketch.new", json!({"body": body.0.to_string()}));
        ok(
            &mut doc,
            "sketch.external_from",
            json!({"sketch": third, "from": first, "counts": true}),
        );
        let counted = sketch_of(&doc, &third);
        assert!(counted.external.values().all(|s| s.defining));
        let wires = crate::profile::extract_wires(&counted).expect("the rectangle counts");
        assert_eq!(wires.len(), 1);
        let corner = copied.geometry.iter().any(|g| matches!(
            g,
            GeometryElement::Point(p) if (p.position.to_glam() - glam::DVec2::new(10.0, 6.0)).length() < 1e-4
        ));
        assert!(
            corner,
            "the far corner lands where it is in the other sketch"
        );

        // A datum plane standing on the XZ plane crosses this one along x.
        let datum = doc
            .add_feature_in_body(
                DatumFeature {
                    shape: DatumShape::Plane { size: 20.0 },
                    attachment: DatumAttachment::BasePlane(BasePlane::XZ),
                    offset: AttachmentOffset::default(),
                },
                "Datum".into(),
                Some(body),
            )
            .unwrap();
        ok(
            &mut doc,
            "sketch.external_from",
            json!({"sketch": second, "from": datum.0.to_string()}),
        );
        let with_datum = sketch_of(&doc, &second);
        let crossing = with_datum.geometry.iter().any(|g| matches!(
            g,
            GeometryElement::Point(p) if (p.position.to_glam() - glam::DVec2::new(10.0, 0.0)).length() < 1e-3
        ));
        assert!(
            crossing,
            "the crossing runs along x, 20 long about the origin"
        );
        assert!(
            refused(
                &mut doc,
                "sketch.external_from",
                json!({"sketch": second, "from": second})
            ),
            "not from itself"
        );

        // The first sketch's far corner moves; brought up to it again, the
        // copy follows.
        let first_id = feature_id(&first);
        let mut data = doc.get_feature_data(first_id).unwrap().clone();
        let mut edited = SketchFeature::from_json(&data).unwrap();
        for g in &mut edited.sketch.geometry {
            if let GeometryElement::Point(p) = g
                && (p.position.to_glam() - glam::DVec2::new(10.0, 6.0)).length() < 1e-4
            {
                p.position = crate::sketch::Vec2D::new(12.0, 7.0);
            }
        }
        data = edited.to_json();
        doc.update_feature_data(first_id, data).unwrap();
        let second_id = feature_id(&second);
        let mut feature =
            SketchFeature::from_json(doc.get_feature_data(second_id).unwrap()).unwrap();
        let placed = crate::placed_plane(&feature.plane, &crate::sketch_placement(&doc, second_id));
        for (source, group) in crate::external::groups(&feature.sketch) {
            let projected =
                crate::external_ref::project(&doc, &placed, source.reference.unwrap()).unwrap();
            crate::external::refresh_group(&mut feature.sketch, source, &group, &projected);
        }
        let moved = feature.sketch.geometry.iter().any(|g| matches!(
            g,
            GeometryElement::Point(p) if (p.position.to_glam() - glam::DVec2::new(12.0, 7.0)).length() < 1e-4
        ));
        assert!(moved, "the copy's corner followed");
    }

    fn line_length(sketch: &Sketch, id: &Value) -> f64 {
        let id = Uuid::parse_str(id.as_str().unwrap()).unwrap();
        let Some(GeometryElement::Line(l)) = sketch.get_geometry(id) else {
            panic!("not a line")
        };
        let (a, b) = (
            sketch.point_position(l.start).unwrap(),
            sketch.point_position(l.end).unwrap(),
        );
        (b - a).to_glam().length()
    }

    #[test]
    fn constraints_drive_the_geometry_and_the_status_counts_them() {
        let mut doc = Document::new("t");
        let s = ok(&mut doc, "sketch.new", json!({}));
        let line = ok(
            &mut doc,
            "sketch.line",
            json!({"sketch": s, "x1": 1, "y1": 1, "x2": 9, "y2": 2}),
        );
        let free = ok(&mut doc, "sketch.status", json!({"sketch": s}))["dof"]
            .as_i64()
            .unwrap();
        ok(
            &mut doc,
            "sketch.constrain",
            json!({"sketch": s, "kind": "horizontal", "items": [line]}),
        );
        let dim = ok(
            &mut doc,
            "sketch.constrain",
            json!({"sketch": s, "kind": "dimension", "items": [line], "value": 25}),
        );
        let sketch = sketch_of(&doc, &s);
        assert!((line_length(&sketch, &line) - 25.0).abs() < 1e-3);
        let status = ok(&mut doc, "sketch.status", json!({"sketch": s}));
        assert_eq!(status["dof"].as_i64().unwrap(), free - 2);
        assert_eq!(status["solved"], json!(true));

        ok(
            &mut doc,
            "sketch.set_value",
            json!({"sketch": s, "constraint": dim[0], "value": 40}),
        );
        assert!((line_length(&sketch_of(&doc, &s), &line) - 40.0).abs() < 1e-3);
        let listed = ok(&mut doc, "sketch.constraints", json!({"sketch": s}));
        assert_eq!(listed.as_array().unwrap().len(), 2);

        // The origin takes constraints by name.
        let start = sketch_of(&doc, &s)
            .geometry
            .iter()
            .find_map(|g| match g {
                GeometryElement::Line(l) => Some(l.start),
                _ => None,
            })
            .unwrap();
        ok(
            &mut doc,
            "sketch.constrain",
            json!({"sketch": s, "kind": "coincident", "items": [start.to_string(), "origin"]}),
        );
        let sketch = sketch_of(&doc, &s);
        assert!(sketch.point_position(start).unwrap().to_glam().length() < 1e-3);

        ok(
            &mut doc,
            "sketch.delete",
            json!({"sketch": s, "items": [dim[0]]}),
        );
        assert_eq!(sketch_of(&doc, &s).constraints.len(), 2);
        assert!(
            refused(
                &mut doc,
                "sketch.constrain",
                json!({"sketch": s, "kind": "parallel", "items": [line]})
            ),
            "parallel takes two lines"
        );
    }

    #[test]
    fn a_sketch_takes_a_plane_of_its_own() {
        let mut doc = Document::new("t");
        let s = ok(
            &mut doc,
            "sketch.new",
            json!({"normal": [1, 0, 0], "origin": [5, 0, 0], "x_axis": [0, 1, 0]}),
        );
        let plane = sketch_of(&doc, &s).plane;
        assert_eq!(plane.origin, [5.0, 0.0, 0.0]);
        assert_eq!(plane.x_axis, [0.0, 1.0, 0.0]);
        assert_eq!(plane.y_axis, [0.0, 0.0, 1.0]);
        assert!(refused(
            &mut doc,
            "sketch.new",
            json!({"normal": [0, 0, 0]})
        ));
    }

    #[test]
    fn bad_arguments_are_refused() {
        let mut doc = Document::new("t");
        let sketch = ok(&mut doc, "sketch.new", json!({}));
        assert!(refused(
            &mut doc,
            "sketch.circle",
            json!({"sketch": sketch, "x": 0, "y": 0, "radius": -1})
        ));
        assert!(refused(&mut doc, "sketch.new", json!({"plane": "AB"})));
        let not_a_sketch = Uuid::new_v4().to_string();
        assert!(refused(
            &mut doc,
            "sketch.point",
            json!({"sketch": not_a_sketch, "x": 0, "y": 0})
        ));
    }

    fn registry() -> core_document::DocumentService {
        let mut registry = core_document::DocumentService::default();
        registry
            .register_workbench(Box::new(crate::SketchWorkbench::default()))
            .unwrap();
        registry
    }

    /// The radius of the sketch's one circle, as `sketch.geometry` reads it.
    fn listed_radius(doc: &mut Document, sketch: &Value) -> f64 {
        let listed = ok(doc, "sketch.geometry", json!({"sketch": sketch}));
        listed
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["kind"] == "circle")
            .and_then(|g| g["radius"].as_f64())
            .unwrap()
    }

    /// A sketch whose projected line's source is deleted warns of it on its
    /// row, through the registry as the tree asks.
    #[test]
    fn a_projection_whose_source_is_gone_warns() {
        let registry = registry();
        let mut doc = Document::new("t");
        let base = ok(&mut doc, "sketch.new", json!({}));
        let line = ok(
            &mut doc,
            "sketch.line",
            json!({"sketch": base, "x1": 0, "y1": 0, "x2": 5, "y2": 1}),
        );
        let top = ok(&mut doc, "sketch.new", json!({"offset": 10}));
        ok(
            &mut doc,
            "sketch.external_from",
            json!({"sketch": top, "from": base}),
        );
        let warning = |doc: &mut Document| {
            registry.evaluate(doc);
            let id = feature_id(&top);
            let node = doc.feature_tree().get_node(id).unwrap().clone();
            registry.feature_warning(doc, None, id, &node)
        };
        assert_eq!(warning(&mut doc), None, "its source stands");

        ok(
            &mut doc,
            "sketch.delete",
            json!({"sketch": base, "items": [line]}),
        );
        let said = warning(&mut doc).expect("a warning");
        assert!(said.starts_with("1 projected element"), "{said}");
    }

    /// A circle copied from another sketch takes the source's new radius
    /// when the document is worked out, set by hand or by a formula, with
    /// the copy never opened; its own stored data is the projection it was
    /// made with.
    #[test]
    fn geometry_from_another_sketch_follows_it_without_the_sketch_being_opened() {
        let registry = registry();
        let mut doc = Document::new("t");
        let base = ok(&mut doc, "sketch.new", json!({}));
        let circle = ok(
            &mut doc,
            "sketch.circle",
            json!({"sketch": base, "x": 0, "y": 0, "radius": 2}),
        );
        let dim = ok(
            &mut doc,
            "sketch.constrain",
            json!({"sketch": base, "kind": "radius", "items": [circle], "value": 2}),
        )[0]
        .clone();
        let top = ok(&mut doc, "sketch.new", json!({"offset": 10}));
        ok(
            &mut doc,
            "sketch.external_from",
            json!({"sketch": top, "from": base, "counts": true}),
        );
        registry.evaluate(&mut doc);
        assert!((listed_radius(&mut doc, &top) - 2.0).abs() < 1e-5);

        ok(
            &mut doc,
            "sketch.set_value",
            json!({"sketch": base, "constraint": dim, "value": 3}),
        );
        registry.evaluate(&mut doc);
        assert!((listed_radius(&mut doc, &top) - 3.0).abs() < 1e-5);
        let built = crate::stored_sketch(&doc, feature_id(&top)).unwrap();
        let wires = crate::profile::extract_wires(&built.sketch).unwrap();
        assert_eq!(wires.len(), 1, "the copy is the profile");

        doc.set_feature_formula(
            feature_id(&base),
            dim.as_str().unwrap(),
            Some("4 mm".into()),
        )
        .unwrap();
        registry.evaluate(&mut doc);
        assert!((listed_radius(&mut doc, &base) - 4.0).abs() < 1e-5);
        assert!((listed_radius(&mut doc, &top) - 4.0).abs() < 1e-5);
        let listed = ok(&mut doc, "sketch.constraints", json!({"sketch": base}));
        assert!((listed[0]["value"].as_f64().unwrap() - 4.0).abs() < 1e-5);
        // Derived: the copy's own data is as it was made.
        assert!(
            sketch_of(&doc, &top)
                .geometry
                .iter()
                .any(|g| matches!(g, GeometryElement::Circle(c) if (c.radius - 2.0).abs() < 1e-5))
        );
    }

    /// The `elements` an external call answers with go back to
    /// `sketch.external_defining` as they come, its curves' end points with
    /// them; anything else is still refused.
    #[test]
    fn external_elements_are_made_to_count_as_they_were_returned() {
        let mut doc = Document::new("t");
        let base = ok(&mut doc, "sketch.new", json!({}));
        ok(
            &mut doc,
            "sketch.rect",
            json!({"sketch": base, "x": 0, "y": 0, "width": 4, "height": 4}),
        );
        let top = ok(&mut doc, "sketch.new", json!({"offset": 10}));
        let made = ok(
            &mut doc,
            "sketch.external_from",
            json!({"sketch": top, "from": base}),
        );
        assert!(crate::profile::extract_wires(&sketch_of(&doc, &top)).is_err());
        ok(
            &mut doc,
            "sketch.external_defining",
            json!({"sketch": top, "items": made["elements"]}),
        );
        assert_eq!(
            crate::profile::extract_wires(&sketch_of(&doc, &top))
                .unwrap()
                .len(),
            1
        );
        let own = ok(
            &mut doc,
            "sketch.point",
            json!({"sketch": top, "x": 20, "y": 20}),
        );
        assert!(refused(
            &mut doc,
            "sketch.external_defining",
            json!({"sketch": top, "items": [own]})
        ));
        // The same sketch again brings nothing twice.
        let again = call(
            &mut doc,
            "sketch.external_from",
            json!({"sketch": top, "from": base}),
        );
        assert!(again.is_err(), "{again:?}");
    }

    /// A radius turned into a diameter keeps its id and the circle's size,
    /// its formula doubled; turned back, the formula is what it was.
    #[test]
    fn a_radius_turns_into_a_diameter_and_back() {
        let mut doc = Document::new("t");
        let s = ok(&mut doc, "sketch.new", json!({}));
        let circle = ok(
            &mut doc,
            "sketch.circle",
            json!({"sketch": s, "x": 0, "y": 0, "radius": 2}),
        );
        let dim = ok(
            &mut doc,
            "sketch.constrain",
            json!({"sketch": s, "kind": "radius", "items": [circle], "value": 2}),
        )[0]
        .clone();
        let key = dim.as_str().unwrap().to_string();
        doc.set_feature_formula(feature_id(&s), &key, Some("1 mm + 1 mm".into()))
            .unwrap();
        let switch = |doc: &mut Document, to: &str| {
            call(
                doc,
                "sketch.set_constraint",
                json!({"sketch": s, "items": [dim], "as": to}),
            )
        };
        switch(&mut doc, "diameter").unwrap();
        let held = sketch_of(&doc, &s)
            .constraints
            .iter()
            .find(|c| c.id.to_string() == key)
            .map(|c| c.kind.clone());
        assert!(
            matches!(held, Some(ConstraintKind::Diameter { diameter, .. }) if (diameter - 4.0).abs() < 1e-5),
            "{held:?}"
        );
        assert_eq!(
            doc.feature_formula(feature_id(&s), &key),
            Some("(1 mm + 1 mm) * 2")
        );
        switch(&mut doc, "radius").unwrap();
        assert_eq!(
            doc.feature_formula(feature_id(&s), &key),
            Some("1 mm + 1 mm")
        );
        assert!(switch(&mut doc, "chord").is_err());
        let line = ok(
            &mut doc,
            "sketch.line",
            json!({"sketch": s, "x1": 0, "y1": 0, "x2": 5, "y2": 1}),
        );
        let length = ok(
            &mut doc,
            "sketch.constrain",
            json!({"sketch": s, "kind": "distance", "items": [line], "value": 5}),
        );
        let refused = call(
            &mut doc,
            "sketch.set_constraint",
            json!({"sketch": s, "items": length, "as": "diameter"}),
        );
        assert!(refused.is_err(), "a length is no circle's size");
    }

    /// Text in a font file named by its path keeps the font in the
    /// document: changed after the file is gone, it is made again from the
    /// kept copy, and a second text in the same file keeps no second one.
    #[test]
    fn a_font_file_is_kept_in_the_document() {
        let path = std::env::temp_dir().join(format!("printcad-font-{}.ttf", Uuid::new_v4()));
        std::fs::write(&path, crate::text::FONTS[2].1).unwrap();
        let font = path.to_string_lossy().to_string();
        let mut doc = Document::new("t");
        let s = ok(&mut doc, "sketch.new", json!({}));
        let fonts = |doc: &Document| {
            doc.assets()
                .filter(|a| a.metadata.get("kind").and_then(|k| k.as_str()) == Some("font"))
                .count()
        };
        let made = ok(
            &mut doc,
            "sketch.text",
            json!({"sketch": s, "text": "A", "at": [0, 0], "font": font}),
        );
        assert_eq!(fonts(&doc), 1);
        std::fs::remove_file(&path).unwrap();
        call(
            &mut doc,
            "sketch.text_edit",
            json!({"sketch": s, "block": made["text"], "text": "AB", "size": 14}),
        )
        .expect("made again from the kept font");
        let block = sketch_of(&doc, &s).texts[0].clone();
        assert_eq!((block.text.as_str(), block.size), ("AB", 14.0));
        assert!(block.font_asset.is_some());

        std::fs::write(&path, crate::text::FONTS[2].1).unwrap();
        ok(
            &mut doc,
            "sketch.text",
            json!({"sketch": s, "text": "C", "at": [0, 20], "font": font}),
        );
        assert_eq!(fonts(&doc), 1, "the same file is kept once");
        std::fs::remove_file(&path).unwrap();
        let missing = call(
            &mut doc,
            "sketch.text",
            json!({"sketch": s, "text": "D", "at": [0, 40], "font": "/nowhere/font.ttf"}),
        );
        assert!(
            missing
                .unwrap_err()
                .to_string()
                .contains("cannot read the font")
        );
    }

    /// A font found installed by its family is kept in the document under
    /// the family the font gives, and text in it is made again from the
    /// kept copy once the document is read back where no such font is
    /// installed.
    #[test]
    fn an_installed_font_is_kept_and_reopens_without_it() {
        const FAMILY: &str = "Printcad Kept Family";
        let installed = |family: &str| {
            family
                .eq_ignore_ascii_case(FAMILY)
                .then(|| crate::text::InstalledFont {
                    family: FAMILY.to_string(),
                    bytes: crate::text::FONTS[2].1.to_vec(),
                    index: 0,
                    file: Some("kept.ttf".to_string()),
                })
        };
        let mut doc = Document::new("t");
        let s = ok(&mut doc, "sketch.new", json!({}));
        let mut spec = crate::text::TextSpec {
            text: "A".to_string(),
            font: "printcad kept family".to_string(),
            ..Default::default()
        };
        crate::text::load_font_from(&mut doc, &mut spec, &installed).unwrap();
        assert_eq!(spec.font, FAMILY, "the family as the font names it");
        let asset = spec.font_asset.expect("kept");
        assert_eq!(
            doc.get_asset(asset).unwrap().metadata["family"],
            json!(FAMILY)
        );
        let mut args = crate::text_args(&spec, None);
        args["sketch"] = s.clone();
        args["at"] = json!([0, 0]);
        let made = ok(&mut doc, "sketch.text", args);

        let bytes = doc.save_to_bytes(core_document::Compression::None).unwrap();
        let mut reopened = Document::load_from_bytes(bytes).unwrap();
        assert!(crate::text::installed_font(FAMILY).is_none());
        call(
            &mut reopened,
            "sketch.text_edit",
            json!({"sketch": s, "block": made["text"], "text": "AB"}),
        )
        .expect("made again from the kept font");
        let block = sketch_of(&reopened, &s).texts[0].clone();
        assert_eq!(block.text, "AB");
        assert_eq!(block.font, FAMILY);
        assert_eq!(block.font_asset, Some(asset));

        let missing = call(
            &mut reopened,
            "sketch.text",
            json!({"sketch": s, "text": "C", "at": [0, 20], "font": FAMILY}),
        );
        assert!(
            missing
                .unwrap_err()
                .to_string()
                .contains("is installed here"),
            "a family neither installed nor kept is refused with why"
        );
    }

    /// A text block's size set by a variable: the outlines the document
    /// builds from are made again at its value, keeping their elements,
    /// and the block's stored data is left as it was made.
    #[test]
    fn a_variable_sets_a_texts_size() {
        let registry = registry();
        let mut doc = Document::new("t");
        let s = ok(&mut doc, "sketch.new", json!({}));
        let made = ok(
            &mut doc,
            "sketch.text",
            json!({"sketch": s, "text": "I", "at": [0, 0], "size": 10}),
        );
        let block = made["text"].as_str().unwrap().to_string();
        let sizes = doc.add_variable_set("Label").unwrap();
        doc.set_variable(sizes, "height", "10 mm", None).unwrap();
        doc.set_feature_formula(
            feature_id(&s),
            format!("{block}/size"),
            Some("Label.height".into()),
        )
        .unwrap();
        let height = |doc: &Document| {
            let built = crate::stored_sketch(doc, feature_id(&s)).unwrap().sketch;
            let ys: Vec<f64> = crate::text::outline_points(&built)
                .iter()
                .filter_map(|p| built.point_position(*p))
                .map(|p| p.y)
                .collect();
            let elements = built.texts[0].elements.clone();
            let top = ys.iter().copied().fold(f64::MIN, f64::max);
            let bottom = ys.iter().copied().fold(f64::MAX, f64::min);
            (top - bottom, elements)
        };
        registry.evaluate(&mut doc);
        let (small, elements) = height(&doc);
        doc.set_variable(sizes, "height", "25 mm", None).unwrap();
        registry.evaluate(&mut doc);
        let (large, kept) = height(&doc);
        assert!((large / small - 2.5).abs() < 1e-6, "{small} then {large}");
        assert_eq!(kept, elements, "the same elements, reshaped");
        let stored = sketch_of(&doc, &s).texts[0].clone();
        assert_eq!(stored.size, 10.0, "the stored data is as it was made");
    }

    #[test]
    fn constraint_kinds_and_flags_are_checked() {
        let mut doc = Document::new("t");
        let s = ok(&mut doc, "sketch.new", json!({}));
        let line = ok(
            &mut doc,
            "sketch.line",
            json!({"sketch": s, "x1": 0, "y1": 0, "x2": 5, "y2": 1}),
        );
        let unknown = call(
            &mut doc,
            "sketch.constrain",
            json!({"sketch": s, "kind": "bogus", "items": [line]}),
        )
        .unwrap_err()
        .to_string();
        assert!(
            unknown.contains("no constraint") && unknown.contains("horizontal"),
            "{unknown}"
        );
        let flat = ok(
            &mut doc,
            "sketch.constrain",
            json!({"sketch": s, "kind": "horizontal", "items": [line]}),
        );
        let set = |doc: &mut Document, args: Value| {
            let mut args = args;
            args["sketch"] = s.clone();
            call(doc, "sketch.set_constraint", args)
        };
        assert!(set(&mut doc, json!({"items": [line], "active": false})).is_err());
        assert!(set(&mut doc, json!({"items": flat, "driving": false})).is_err());
        assert!(set(&mut doc, json!({"items": flat, "active": false})).is_ok());
    }

    /// Two lengths for one line: the sketch does not solve, says so, and
    /// gives no profile to build from until one goes.
    #[test]
    fn a_sketch_whose_constraints_conflict_gives_no_profile() {
        let mut doc = Document::new("t");
        let s = ok(&mut doc, "sketch.new", json!({}));
        let sides = ok(
            &mut doc,
            "sketch.rect",
            json!({"sketch": s, "x": 0, "y": 0, "width": 4, "height": 4}),
        );
        let length = |doc: &mut Document, value: f64| {
            ok(
                doc,
                "sketch.constrain",
                json!({"sketch": s, "kind": "distance", "items": [sides[0]], "value": value}),
            )
        };
        length(&mut doc, 4.0);
        let second = length(&mut doc, 8.0);
        let status = ok(&mut doc, "sketch.status", json!({"sketch": s}));
        assert_eq!(status["solved"], false);
        let why = crate::profile::extract_wires(&sketch_of(&doc, &s)).unwrap_err();
        assert!(why.to_string().contains("does not solve"), "{why}");
        ok(
            &mut doc,
            "sketch.delete",
            json!({"sketch": s, "items": second}),
        );
        assert!(crate::profile::extract_wires(&sketch_of(&doc, &s)).is_ok());
    }
}
