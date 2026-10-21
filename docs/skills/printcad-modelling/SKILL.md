---
name: printcad-modelling
description: Model parts in printCAD through its MCP tools (context, search, describe, call, batch, lua, view). Use when asked to make, change or check a part, sketch, hole, fillet or assembly in printCAD; it covers finding the right commands, building in small steps, and proving the result by measuring it and looking at it.
---

# Modelling in printCAD

printCAD's tools reach the document the user has open. Everything you do
is a command (`pc.<id>{...}` in Lua, or `call`/`batch`); the user sees
each change, and each `call`, `batch` or `lua` is one undo step.

## The loop

1. **Know where you are.** Call `context`: the open document, its bodies,
   what is selected and being edited, and the rules to keep (wall
   thicknesses, units, names). Follow the rules over your own habits.
2. **Find the commands.** `search` with the words of the task ("hole
   through", "fillet edge", "pattern around"); `describe` a command
   before using it the first time, for its arguments, notes and a working
   example. The recipes (`docs/recipes/`) are whole parts as scripts:
   start from the nearest one.
3. **Build in steps.** One sketch and the feature it drives per step, in
   `lua` or `batch`, each step something you can check. Name features
   after what they are for (`name = "Mounting holes"`).
4. **Rebuild and read the failures.** End each step with
   `pc.doc.rebuild()`: it answers `{feature, error}` for each feature that
   failed. A failed step is taken back whole when a script stops on an
   error: `assert(#pc.doc.rebuild() == 0)` makes it do so.
5. **Prove it.** Work out what the part should measure before you build
   it, then compare: `pc.doc.measure{body = ...}` (volume, area, centre),
   `pc.doc.faces{body = ...}` (each face's kind and radius),
   `pc.doc.edges{body = ...}`. Assert the numbers in the script, as the
   recipes do.
6. **Look.** `view` with a fixed view (`"iso"`, `"front"`, ...) and the
   bodies you made. The picture is the document as built; an open
   sketch's lines and surface textures are not in it.
7. **Report** what you made, the numbers that prove it, and anything left
   for the user to decide.

## What trips people up

- **Planes face a fixed way.** XY faces +Z, XZ faces -Y and YZ faces +X.
  A pad grows along its sketch's normal; `reversed = true` grows it the
  other way. A pocket and a hole cut against the normal, into a face a
  sketch sits on.
- **A sketch on a body's face** is started in that body
  (`pc.sketch.new{body = body, plane = "XY", offset = t}`), so the
  feature it drives lands in the same body.
- **Sketch coordinates** run along the plane's axes: on YZ, the sketch's
  x is world Y and its y world Z.
- **Holes** take a sketch's circle centres and points; a circle's radius
  is ignored, the hole's `diameter` sets the size.
- **Edges** for fillets and chamfers are named by a point on the edge and
  the way it runs: `edges = {Edges = {{point = {x, y, z}, direction =
  {0, 1, 0}}}}`. Read `pc.doc.edges{body = ...}` when unsure.
- **Formulas** keep a part adjustable: make a variable set
  (`pc.var.new`) and set a dimension from it (`pc.doc.set_formula`)
  rather than baking a number in twice.
- **Files** (import, export, pictures to a file) take whole paths inside
  the document's folder or the temporary folder, and wait for the user's
  OK, one `call` at a time.

## Before calling a part done

- Every feature builds (`#pc.doc.rebuild() == 0`).
- Its measures match what you worked out.
- Walls and features meet the document's rules (a wall at least the
  thickness the rules ask; holes the sizes asked).
- A picture from a fixed view shows what was asked.
