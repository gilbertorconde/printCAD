//! Text laid out as sketch geometry, and changing it.

use core_document::{
    Args, CommandArgs, CommandError, CommandResult, CommandSpec, FeatureId, ParamKind,
    WorkbenchFeature, WorkbenchRuntimeContext,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{load, points, sketch};

/// Whether `id` is one of these commands.
pub(super) fn owns(id: &str) -> bool {
    matches!(
        id,
        "sketch.text" | "sketch.text_edit" | "sketch.explode_text"
    )
}

/// These commands' specs.
pub(super) fn configure(specs: &mut Vec<CommandSpec>) {
    specs.push(
        sketch(CommandSpec::new(
            "sketch.text",
            "Write text from a point, in a frame or along curves: one object whose letters \
             are closed outlines",
        ))
        .param(
            "text",
            ParamKind::String,
            "What it says; a new line starts a line",
        )
        .optional(
            "at",
            ParamKind::List,
            "Where its point goes, {x, y}: needed unless `frame` or `path` is given",
        )
        .optional(
            "frame",
            ParamKind::List,
            "Two corners across a frame to write in, {{x, y}, {x, y}}",
        )
        .optional(
            "path",
            ParamKind::List,
            "Curves to write along, end to end, {id, ...}; `at` then picks where on them \
             the text stands (their start)",
        )
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
             with why.",
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
        .optional(
            "align",
            ParamKind::String,
            "start, middle or end: where each line stands along its room, or along the \
             curve from the point (start)",
        )
        .optional(
            "valign",
            ParamKind::String,
            "baseline, top, middle or bottom: where the lines stand up and down from the \
             point or in the frame (baseline: on the point, at the frame's top)",
        )
        .optional(
            "line_spacing",
            ParamKind::Number,
            "From one baseline to the next, a share of the font's own (1)",
        )
        .optional(
            "wrap",
            ParamKind::Bool,
            "In a frame, lines break between words to keep inside it (false)",
        )
        .optional(
            "fit",
            ParamKind::Bool,
            "In a frame, text larger than it shrinks to fit; along curves, the letters \
             spread to fill them (false)",
        )
        .optional(
            "side",
            ParamKind::String,
            "above, below or centred: where the letters stand on the curve (above: on it, \
             to its left as it runs)",
        )
        .optional(
            "offset",
            ParamKind::Number,
            "Along curves, mm from the point the text stands (0)",
        )
        .optional(
            "reversed",
            ParamKind::Bool,
            "Along curves, walk them the other way (false)",
        )
        .returns(
            "{text, point, corners}: the block, the point it stands on and, in a frame, its \
             four corners from the bottom left",
        )
        .note(
            "A closed curve runs clockwise from its top, so text above it stands outside and \
             reads across the top; `reversed` puts it inside, reading across the bottom. \
             The point is held on the curve: dragging it carries the text along.",
        )
        .note(
            "A frame is four construction lines kept square to the sketch's axes; dragging \
             or dimensioning them sizes the room the text has.",
        )
        .note(
            "`{formula}` in the text shows what the formula comes to, and follows it: \
             `REV {Part.rev}` reads the variable, `{Width * 2:1}` writes one decimal. A \
             length shows in millimetres, an angle in degrees; `{{` and `}}` are braces. \
             Each formula is kept as the sketch's under `<block>/value/<n>`.",
        )
        .note(
            "`at` is where the baseline starts: the letters stand on it, capitals reaching \
             about 0.7 of `size`, the em.",
        )
        .note(
            "The letters are closed outlines that pad as they read; dragging the point \
             returned moves the whole text. They are made from the block, never stored as \
             curves of their own, so the text stays editable with `sketch.text_edit`; \
             `sketch.explode_text` turns it into plain curves.",
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
        )
        .example(
            "A revision label that follows a variable",
            r#"
            pc.var.new{name = "Part"}
            pc.var.set{set = "Part", name = "rev", formula = "3"}
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.text{sketch = s, text = "REV {Part.rev}", at = {0, 0}}
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local body = pc.doc.feature{id = pad}.body
            local before = pc.doc.measure{body = body}
            pc.var.set{set = "Part", name = "rev", formula = "12"}
            assert(#pc.doc.rebuild() == 0)
            local after = pc.doc.measure{body = body}
            assert(after.max[1] > before.max[1] + 3, "a digit longer")
            "#,
        )
        .example(
            "A label centred in a frame",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local t = pc.sketch.text{sketch = s, text = "OK", frame = {{0, 0}, {40, 20}},
              align = "middle", valign = "middle"}
            assert(#t.corners == 4)
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(math.abs((m.min[1] + m.max[1]) / 2 - 20) < 0.5, "centred across")
            assert(math.abs((m.min[2] + m.max[2]) / 2 - 10) < 0.5, "centred up and down")
            "#,
        )
        .example(
            "Letters round a circle",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            -- A guide circle: the letters are the profile.
            local ring = pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 20}
            pc.sketch.construction{sketch = s, items = {ring}}
            pc.sketch.text{sketch = s, text = "AROUND", path = {ring}, at = {0, 30},
              align = "middle", size = 5}
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
            assert(m.min[2] > 15, "over the circle's top")
            assert(math.abs(m.min[1] + m.max[1]) < 1, "centred on the point")
            "#,
        ),
    );
    specs.push(
        sketch(CommandSpec::new(
            "sketch.text_edit",
            "Change a text block: what it says, its font, size, spacing or angle",
        ))
        .param("block", ParamKind::Id, "The text block, or its point")
        .optional("text", ParamKind::String, "")
        .optional("font", ParamKind::String, "")
        .optional("font_asset", ParamKind::String, "")
        .optional("size", ParamKind::Number, "mm")
        .optional("spacing", ParamKind::Number, "mm")
        .optional("angle", ParamKind::Number, "degrees")
        .optional("align", ParamKind::String, "start, middle or end")
        .optional(
            "valign",
            ParamKind::String,
            "baseline, top, middle or bottom",
        )
        .optional("line_spacing", ParamKind::Number, "")
        .optional("wrap", ParamKind::Bool, "In a frame")
        .optional("fit", ParamKind::Bool, "In a frame or along curves")
        .optional(
            "side",
            ParamKind::String,
            "above, below or centred, along curves",
        )
        .optional("offset", ParamKind::Number, "mm, along curves")
        .optional("reversed", ParamKind::Bool, "Along curves")
        .note("`block` is either the `text` or the `point` that `sketch.text` returned.")
        .note(
            "Only what is given changes, and the outlines are made again on the same point; \
             a `size` of 0 or less is refused.",
        )
        .note(
            "A block's size, spacing and angle are numbers formulas can set: \
             `pc.doc.parameters` of the sketch lists them under the keys `<block>/size`, \
             `<block>/spacing` and `<block>/angle`, and the letters stand at what they come \
             to. The string is not, since a formula makes a number.",
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
    specs.push(
        sketch(CommandSpec::new(
            "sketch.explode_text",
            "Turn a text block into plain lines and splines, which can then be edited one by \
             one",
        ))
        .param("block", ParamKind::Id, "The text block, or its point")
        .returns("{curves}: the lines and splines the letters left")
        .note(
            "The curves keep the names the letters had, so what was built from the text \
             keeps its faces' names; the point stays, and the text is gone for good.",
        )
        .see_also("sketch.text")
        .example(
            "A letter exploded and still padded",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            local t = pc.sketch.text{sketch = s, text = "L", at = {0, 0}, size = 10}
            local pad = pc.design.pad{sketch = s, length = 1}
            assert(#pc.doc.rebuild() == 0)
            local body = pc.doc.feature{id = pad}.body
            local before = pc.doc.measure{body = body}.volume
            local left = pc.sketch.explode_text{sketch = s, block = t.text}
            assert(#left.curves > 0)
            assert(#pc.doc.rebuild() == 0)
            assert(math.abs(pc.doc.measure{body = body}.volume - before) < 1e-6)
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
    let mut exploded = None;
    let answer = match id {
        "sketch.text" => {
            let mut spec = crate::text::TextSpec {
                text: a.string("text")?.to_string(),
                ..Default::default()
            };
            if a.has("frame") {
                spec.layout = crate::text::TextLayout::Frame {
                    corners: [Uuid::nil(); 4],
                    wrap: false,
                    fit: false,
                };
            } else if a.has("path") {
                spec.layout = crate::text::TextLayout::Path {
                    curves: Vec::new(),
                    side: Default::default(),
                    offset: 0.0,
                    fit: false,
                    reversed: false,
                };
            }
            text_options(&a, &mut spec)?;
            crate::text::load_font(ctx.document, &mut spec).map_err(CommandError::failed)?;
            spec.values =
                crate::text::values_of(ctx.document, &spec.text).map_err(CommandError::failed)?;
            let at = match args.get("at") {
                Some(at) => Some(
                    points(Some(&json!([at])))
                        .map_err(|_| CommandError::bad("at", "must be {x, y}"))?[0],
                ),
                None => None,
            };
            let block = if a.has("frame") {
                let corners = points(args.get("frame"))
                    .ok()
                    .filter(|c| c.len() == 2)
                    .ok_or_else(|| {
                        CommandError::bad("frame", "must be two corners, {{x, y}, {x, y}}")
                    })?;
                crate::text::add_in_frame(sketch, corners[0], corners[1], &spec)
            } else if a.has("path") {
                let curves = super::ids(args.get("path"), "path", sketch)?;
                crate::text::add_on_path(sketch, &curves, at, &spec)
            } else {
                let at = at.ok_or_else(|| {
                    CommandError::bad("at", "is needed without a frame or a path")
                })?;
                crate::text::add(sketch, at, &spec)
            }
            .map_err(CommandError::failed)?;
            let made = sketch.texts.iter().find(|b| b.id == block);
            let mut answer = json!({
                "text": block.to_string(),
                "point": made.map(|b| b.anchor).unwrap_or_default().to_string(),
            });
            if let Some(crate::text::TextLayout::Frame { corners, .. }) = made.map(|b| &b.layout) {
                answer["corners"] = json!(corners.map(|c| c.to_string()));
            }
            answer
        }
        "sketch.explode_text" => {
            let id = block_arg(args, sketch)?;
            let left = crate::text::explode(sketch, id).map_err(CommandError::failed)?;
            exploded = Some(id);
            let curves: Vec<String> = left
                .iter()
                .filter(|e| {
                    !matches!(
                        sketch.get_geometry(**e),
                        Some(crate::sketch::GeometryElement::Point(_)) | None
                    )
                })
                .map(Uuid::to_string)
                .collect();
            json!({"curves": curves})
        }
        "sketch.text_edit" => {
            let id = block_arg(args, sketch)?;
            let block = sketch
                .texts
                .iter()
                .find(|b| b.id == id)
                .expect("found above");
            let mut spec = crate::text::TextSpec::of(block);
            if let Some(t) = a.opt_string("text")? {
                spec.text = t.to_string();
                spec.values = crate::text::values_of(ctx.document, &spec.text)
                    .map_err(CommandError::failed)?;
            }
            text_options(&a, &mut spec)?;
            crate::text::load_font(ctx.document, &mut spec).map_err(CommandError::failed)?;
            crate::text::change(sketch, id, &spec).map_err(CommandError::failed)?;
            Value::Null
        }
        _ => return Err(CommandError::Unknown(id.to_string())),
    };
    crate::solver::solve(sketch);
    let kept: Vec<(Uuid, String)> = sketch
        .texts
        .iter()
        .map(|b| (b.id, b.text.clone()))
        .collect();
    ctx.document
        .update_feature_data(sketch_id, feature.to_json())
        .map_err(|e| CommandError::failed(e.to_string()))?;
    // Each text's formulas set its values from now on; an exploded one's
    // go with it.
    for (block, text) in kept.into_iter().chain(exploded.map(|b| (b, String::new()))) {
        crate::text::keep_formulas(ctx.document, sketch_id, block, &text)
            .map_err(CommandError::failed)?;
    }
    ctx.document.mark_feature_dirty(sketch_id);
    Ok(answer)
}

/// The text block `block` names, by its own id or its point's.
fn block_arg(args: &CommandArgs, sketch: &crate::sketch::Sketch) -> Result<Uuid, CommandError> {
    let named = args
        .get("block")
        .and_then(Value::as_str)
        .and_then(|t| Uuid::parse_str(t).ok())
        .ok_or_else(|| CommandError::bad("block", "must be a text block's id"))?;
    sketch
        .texts
        .iter()
        .find(|b| b.id == named || b.anchor == named)
        .map(|b| b.id)
        .ok_or_else(|| CommandError::bad("block", "is no text block"))
}

/// A text's font, size, spacing, angle and layout settings where `a`
/// gives them.
fn text_options(a: &Args, spec: &mut crate::text::TextSpec) -> Result<(), CommandError> {
    use crate::text::{Align, PathSide, TextLayout, VAlign};
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
    if let Some(v) = a.opt_number("line_spacing")? {
        if v <= 0.0 {
            return Err(CommandError::bad("line_spacing", "must be more than 0"));
        }
        spec.line_spacing = v;
    }
    if let Some(v) = a.opt_string("align")? {
        spec.align = match v.to_ascii_lowercase().as_str() {
            "start" => Align::Start,
            "middle" | "centre" | "center" => Align::Middle,
            "end" => Align::End,
            _ => return Err(CommandError::bad("align", "must be start, middle or end")),
        };
    }
    if let Some(v) = a.opt_string("valign")? {
        spec.valign = match v.to_ascii_lowercase().as_str() {
            "baseline" => VAlign::Baseline,
            "top" => VAlign::Top,
            "middle" | "centre" | "center" => VAlign::Middle,
            "bottom" => VAlign::Bottom,
            _ => {
                return Err(CommandError::bad(
                    "valign",
                    "must be baseline, top, middle or bottom",
                ));
            }
        };
    }
    let (wrap, fit, side, offset, reversed) = (
        a.opt_bool("wrap")?,
        a.opt_bool("fit")?,
        a.opt_string("side")?,
        a.opt_number("offset")?,
        a.opt_bool("reversed")?,
    );
    match &mut spec.layout {
        TextLayout::Frame {
            wrap: w, fit: f, ..
        } => {
            *w = wrap.unwrap_or(*w);
            *f = fit.unwrap_or(*f);
        }
        TextLayout::Path {
            side: s,
            offset: o,
            fit: f,
            reversed: r,
            ..
        } => {
            if let Some(side) = side {
                *s = match side.to_ascii_lowercase().as_str() {
                    "above" => PathSide::Above,
                    "below" => PathSide::Below,
                    "centred" | "centered" | "middle" => PathSide::Centred,
                    _ => return Err(CommandError::bad("side", "must be above, below or centred")),
                };
            }
            *o = offset.unwrap_or(*o);
            *f = fit.unwrap_or(*f);
            *r = reversed.unwrap_or(*r);
        }
        TextLayout::Point => {
            for (name, given) in [
                ("wrap", wrap.is_some()),
                ("fit", fit.is_some()),
                ("side", side.is_some()),
                ("offset", offset.is_some()),
                ("reversed", reversed.is_some()),
            ] {
                if given {
                    return Err(CommandError::bad(
                        name,
                        "is for text in a frame or along curves",
                    ));
                }
            }
        }
    }
    Ok(())
}
