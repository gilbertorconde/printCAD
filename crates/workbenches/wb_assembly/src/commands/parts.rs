//! The parts list: its rows, one part's settings, the print material and the
//! table as text.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, CommandSpec, CommandTable, ParamKind,
    WorkbenchRuntimeContext,
};
use serde_json::{Value, json};

/// These commands, each spec beside what runs it.
pub(super) fn table(t: &mut CommandTable) {
    t.add(
        CommandSpec::new(
            "asm.parts",
            "Every part: bodies of the same shape counted together",
        )
        .optional(
            "by_component",
            ParamKind::Bool,
            "Each component's parts under it: every entry gains a depth, and components \
             come as {component, name, depth}",
        )
        .returns(
            "a list of {name, quantity, bodies, size = {x, y, z} in mm or nil, mesh, number \
             or nil, bought, values = {column = text}, print, volume (mm³) or nil, mass (g) \
             or nil, density, threads = {{designation, count}}}, numbered parts first by \
             number, then by name",
        )
        .read_only()
        .note(
            "Bodies count as one part when they share one shape: linked copies from \
             `asm.copy` do; bodies modelled apart do not, however alike.",
        )
        .note("`size` is the part's bounding box, mm.")
        .note(
            "`threads` are one piece's cosmetic threads (a hole's that shows its thread \
             rather than cutting it), counted by designation such as M6.",
        )
        .note(
            "`volume` and `mass` are of one piece: the volume the kernel measures of its \
             solid (a mesh body's from its triangles), the mass at its body's material's \
             density, else at the list's printing material (`asm.print_material`). `print` \
             is how many to print: the count set with `asm.part`, else one per body, none \
             of a bought part.",
        )
        .see_also("asm.part")
        .see_also("asm.parts_table")
        .see_also("asm.copy")
        .example(
            "A plate and its copies counted as one part",
            r#"
            local function box(x)
              local s = pc.sketch.new{plane = "XY"}
              pc.sketch.rect{sketch = s, x = x, y = 0, width = 10, height = 10}
              return pc.doc.feature{id = pc.design.pad{sketch = s, length = 2}}.body
            end
            local plate = box(0)
            local other = box(20)
            assert(#pc.doc.rebuild() == 0)
            pc.asm.copy{body = plate, count = 2}
            local parts = pc.asm.parts{}
            assert(#parts == 2, "alike bodies modelled apart are two parts")
            local counts = {}
            for _, p in ipairs(parts) do counts[p.quantity] = p end
            assert(counts[3] and counts[3].size[3] == 2,
              "the plate and its two copies: one part, three of it")
            "#,
        ),
        parts,
    );
    t.add(
        CommandSpec::new(
            "asm.part",
            "Set what the parts list keeps for a part: its number, whether it is bought, \
             its values in the added columns",
        )
        .param("body", ParamKind::Id, "Any body of the part")
        .optional("number", ParamKind::Number, "Its item number")
        .optional(
            "bought",
            ParamKind::Bool,
            "Bought rather than made: left out of exports and the slicer",
        )
        .optional(
            "values",
            ParamKind::Any,
            "{column = text}: its values, a column not yet in the list added to it",
        )
        .optional(
            "print",
            ParamKind::Number,
            "How many of it to print; below 0 goes back to one per body",
        )
        .note(
            "What is set is kept for the whole part, every body of its shape, whichever \
             body is named.",
        )
        .note(
            "A `number` of 0 or less takes the number away. A value that is not text is kept \
             as text: 3 becomes \"3\".",
        )
        .see_also("asm.parts")
        .see_also("asm.parts_table")
        .example(
            "A bought screw numbered on one of its copies",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
            local screw = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
            assert(#pc.doc.rebuild() == 0)
            local copies = pc.asm.copy{body = screw, count = 3}
            pc.asm.part{body = copies[2], number = 4, bought = true,
              values = {Supplier = "ACME"}}
            local p = pc.asm.parts{}[1]
            assert(p.quantity == 4 and p.number == 4 and p.bought,
              "set on one, kept for the part")
            assert(p.values.Supplier == "ACME")
            "#,
        ),
        part,
    );
    t.add(
        CommandSpec::new(
            "asm.print_material",
            "Set the material the parts list weighs printed parts in",
        )
        .param(
            "name",
            ParamKind::String,
            "PLA, PETG, ABS, ASA, TPU, Nylon or PC, or a name of your own with its density",
        )
        .optional(
            "density",
            ParamKind::Number,
            "g/cm³; an offered material's own when left out",
        )
        .returns("{name, density}")
        .note(
            "It is kept with the document. A body with a material of its own \
             (`doc.set_body`) is weighed at that material's density instead.",
        )
        .note("A name the list does not offer needs its density.")
        .see_also("asm.parts")
        .see_also("asm.part")
        .example(
            "A plate's filament in PETG, two to print",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
            local plate = pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
            assert(#pc.doc.rebuild() == 0)
            local m = pc.asm.print_material{name = "petg"}
            assert(m.name == "PETG" and math.abs(m.density - 1.27) < 1e-6)
            pc.asm.part{body = plate, print = 2}
            local p = pc.asm.parts{}[1]
            assert(p.print == 2 and math.abs(p.volume - 1000) < 1e-3)
            assert(math.abs(p.mass - 1.27) < 1e-4, "1 cm³ at 1.27 g/cm³")
            "#,
        ),
        print_material,
    );
    t.add(
        CommandSpec::new(
            "asm.parts_table",
            "Replace what the parts list keeps, whole",
        )
        .param(
            "table",
            ParamKind::Any,
            "{columns = {...}, entries = {[body id] = {number, bought, values}}}",
        )
        .note(
            "It replaces the whole list: a number, bought mark or value the table leaves out \
             is gone.",
        )
        .note(
            "An empty Lua table goes as a list and `entries` refuses it: leave `entries` out \
             for none.",
        )
        .see_also("asm.part")
        .see_also("asm.parts")
        .example(
            "The parts list written whole",
            r#"
            local s = pc.sketch.new{plane = "XY"}
            pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
            local screw = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
            assert(#pc.doc.rebuild() == 0)
            pc.asm.part{body = screw, number = 9, values = {Note = "old"}}
            pc.asm.parts_table{table = {
              columns = {"Supplier"},
              entries = {[screw] = {number = 2, bought = true, values = {Supplier = "ACME"}}},
            }}
            local p = pc.asm.parts{}[1]
            assert(p.number == 2 and p.bought and p.values.Supplier == "ACME")
            assert(p.values.Note == nil, "what the table left out is gone")
            "#,
        ),
        parts_table,
    );
}

fn parts(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let mut parts = crate::parts_list(ctx.document, &ctx.bought_kinds);
    let mut volumes = crate::parts::Volumes::default();
    volumes.measure_now(ctx.document, &parts, ctx.kernel);
    volumes.fill(ctx.document, &mut parts);
    let entry = |part: &crate::Part, bodies: &[BodyId]| {
        json!({
            "print": part.print,
            "volume": part.volume_mm3,
            "mass": part.mass_g(),
            "density": part.density,
            "name": part.name,
            "quantity": bodies.len(),
            "bodies": bodies.iter().map(|b| b.0.to_string()).collect::<Vec<_>>(),
            "size": part.size_mm,
            "mesh": part.mesh,
            "number": part.number,
            "bought": part.bought,
            "values": part.values,
            "threads": part
                .threads
                .iter()
                .map(|(designation, count)| json!({"designation": designation, "count": count}))
                .collect::<Vec<_>>(),
        })
    };
    if a.opt_bool("by_component")? != Some(true) {
        return Ok(Value::Array(
            parts.iter().map(|p| entry(p, &p.bodies)).collect(),
        ));
    }
    Ok(Value::Array(
        crate::parts::parts_by_component(ctx.document, &parts)
            .into_iter()
            .map(|row| match row {
                crate::parts::LevelRow::Component { depth, id, name } => {
                    json!({"component": id.0.to_string(), "name": name, "depth": depth})
                }
                crate::parts::LevelRow::Part {
                    depth,
                    part,
                    bodies,
                } => {
                    let mut value = entry(&parts[part], &bodies);
                    value["depth"] = json!(depth);
                    value
                }
            })
            .collect(),
    ))
}

fn print_material(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let name = a.string("name")?.to_string();
    let density = a.opt_number("density")?;
    if density.is_some_and(|d| d <= 0.0) {
        return Err(CommandError::bad("density", "must be above 0"));
    }
    let known = crate::parts::PRINT_MATERIALS
        .iter()
        .any(|(n, _)| n.eq_ignore_ascii_case(&name));
    if !known && density.is_none() {
        return Err(CommandError::bad(
            "density",
            format!("{name} is not a material offered: give its density"),
        ));
    }
    let mut table = crate::parts::table_of(ctx.document)
        .map(|(_, t)| t)
        .unwrap_or_default();
    table.material = crate::parts::PrintMaterial::named(&name, density.map(|d| d as f32));
    crate::parts::store_table(ctx.document, &table)
        .map_err(|e| CommandError::failed(e.to_string()))?;
    Ok(json!({"name": table.material.name, "density": table.material.density}))
}

fn parts_table(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let table: crate::parts::PartsTable =
        serde_json::from_value(a.0.get("table").cloned().unwrap_or(Value::Null))
            .map_err(|e| CommandError::bad("table", e.to_string()))?;
    crate::parts::store_table(ctx.document, &table)
        .map_err(|e| CommandError::failed(e.to_string()))?;
    Ok(Value::Null)
}

fn part(args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    let body = a.body(ctx.document, "body")?;
    let part = crate::parts_list(ctx.document, &ctx.bought_kinds)
        .into_iter()
        .find(|p| p.bodies.contains(&body));
    let bodies = part
        .as_ref()
        .map_or_else(|| vec![body], |p| p.bodies.clone());
    let mut table = crate::parts::table_of(ctx.document)
        .map(|(_, t)| t)
        .unwrap_or_default();
    if let Some(bought) = a.opt_bool("bought")? {
        match &part {
            Some(part) => table.set_bought(part, bought),
            None => table.entry_mut(&bodies).bought = bought,
        }
    }
    let entry = table.entry_mut(&bodies);
    if let Some(n) = a.opt_number("number")? {
        entry.number = n.max(0.0) as u32;
    }
    if let Some(n) = a.opt_number("print")? {
        entry.print = (n >= 0.0).then_some(n.round() as u32);
    }
    if let Some(values) = a.0.get("values").and_then(Value::as_object) {
        for (column, value) in values {
            let text = value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_string);
            entry.values.insert(column.clone(), text);
        }
        for column in values.keys() {
            if !table.columns.contains(column) {
                table.columns.push(column.clone());
            }
        }
    }
    crate::parts::store_table(ctx.document, &table)
        .map_err(|e| CommandError::failed(e.to_string()))?;
    Ok(Value::Null)
}
