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
    matches!(id, "sketch.text" | "sketch.text_edit")
}

/// These commands' specs.
pub(super) fn configure(specs: &mut Vec<CommandSpec>) {
    specs.push(
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
    specs.push(
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
        _ => return Err(CommandError::Unknown(id.to_string())),
    };
    crate::solver::solve(sketch);
    ctx.document
        .update_feature_data(sketch_id, feature.to_json())
        .map_err(|e| CommandError::failed(e.to_string()))?;
    ctx.document.mark_feature_dirty(sketch_id);
    Ok(answer)
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
