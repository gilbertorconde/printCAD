# Scripting

printCAD runs Lua 5.4 scripts. Everything the application and its
workbenches can do by command is a function under `pc`, called with named
arguments in braces:

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 30, height = 20}
local pad = pc.part.pad{sketch = s, length = 12}
pc.part.set{feature = pad, length = 20}
```

## Where scripts run

- **The console.** Windows › Console, Scripts › Console, or the toolbar's
  Scripts button. Enter runs what is typed, Shift+Enter starts another
  line, Tab completes a command name, Up and Down go back through what was
  run. An expression shows its value. Globals stay set between runs, and
  `local` names last one run only. Save as script writes everything run
  in the console since the start as a new script in the scripts folder.
- **Script files.** Scripts › Run script… runs any `.lua` file. Every
  `.lua` file in the scripts folder (`~/.config/printcad/scripts`) is also
  a command of its own: it shows in the Scripts menu, the toolbar's Scripts
  button and the command palette, and takes a key in Preferences ›
  Keyboard. Its first comment line is its description. Scripts › New
  script starts one from a template.
- **The command line.** No window opens:

  ```sh
  printcad --script build.lua --open part.prtcad --save out.prtcad -- 40 20
  ```

  `--open` and `--save` are optional. The words after `--` are the
  script's `arg` table. Output goes to stdout and log lines to stderr. The
  exit code is 0 when the script finishes and 1 when it stops on an error.
  Commands that need a window (the view, the selection, tools) are not
  available there.

## Working with commands

- `help()` lists every command; `help("sketch")` those starting with
  `sketch`. `show(value)` prints a table.
- Ids of bodies, features, sketch elements and constraints are strings.
  Commands that make something answer its id.
- An empty list is `array()`: a plain `{}` reaches a command as an empty
  table of names. `array(1, 2)` is the same as `{1, 2}`.
- A command that fails raises a Lua error with the reason, which stops the
  script. `pcall(pc.part.pad, {sketch = s})` catches it instead.
- Every change is an ordinary edit, so Undo takes it back. A console line
  or a script run is one undo step, and so is anything you edit by hand
  while it runs.
- Solids rebuild after a script, as they do after a click. To read a solid
  in the same script, call `pc.doc.rebuild()` first: it waits and answers
  the features that failed.
- `pc.doc.faces{body = b}` lists a solid's faces with a point on each and
  a flat face's normal or a round face's axis. Assembly joints take those
  faces as they are listed.
- Scripts run on a thread of their own, so the window stays live while
  one runs. The status bar and the console show it with a Stop button.
  Lines and scripts started meanwhile wait their turn. On the command line
  a run stops after an hour.

## Recording

Scripts › Record… (or the toolbar's Scripts button) records what you do
from then on; Stop saves it as `recording_<n>.lua` in the scripts folder,
where the Scripts menu lists it. The status bar shows the recording with a
Stop button.

A click and a script run the same code: every tool ends in the command it
stands for, and the recording is the list of those commands.

- A sketch tool records one `pc.sketch.draw` call per shape: the tool, the
  points its clicks landed on in the sketch, the values typed at them, and
  its settings. A replay runs the same tool over the same points, so it
  snaps and constrains the same way.
- Constraint tools, dimension edits, drags, deletes and construction record
  as `sketch.constrain`, `sketch.set_value`, `sketch.drag`, `sketch.delete`
  and `sketch.construction`.
- A Part Design feature records when its task closes with OK, as the
  command that makes it with the fields that differ from what that command
  makes on its own. An edit records as `part.set` with the fields changed.
- A joint records with its faces where the bodies were before it moved
  them; a move records the placement it ended at.
- The sketcher's other actions record too: arrays, cut and paste (the
  pasted geometry goes into the script), mirrored and merged sketches,
  carbon copies, external geometry, a new plane, driving and active flags.
- Renaming, showing or hiding, suppressing, reordering, moving the tip of
  and deleting tree rows record as `doc.*`, as do Repair shape and Convert
  to solid; an import records as `file.import` with its path; the Solve
  button as `asm.solve`.
- What a recording makes is named (`pad1`, `rect2.elements[3]`), and later
  lines use the name, so a replay works on the things it makes. Things
  that were there before the recording started are named by their id: the
  script expects the same document.
- Undo and Redo are not recorded; what they take back stays in the
  recording. Nor are the view, the selection or a script run meanwhile.

## Variables and formulas

Any number of a feature can be set by a formula over variables and other
objects' dimensions, and follows when they change:

```lua
pc.var.new{name = "Printer"}
pc.var.set{set = "Printer", name = "nozzle", formula = "0.4 mm"}
pc.var.set{set = "Printer", name = "wall", formula = "3 * Printer.nozzle"}
local pad = pc.part.pad{sketch = s, length = 5}
pc.doc.set_formula{id = pad, parameter = "length", formula = "Printer.wall * 10"}
print(pc.var.eval{formula = "Pad.length"}.text)  -- 12 mm
```

`pc.doc.parameters{id = ...}` lists a feature's numbers and what formulas
call them. See [Variables and formulas](VARIABLES.md) for what a formula
can say.

## Examples

A plate with a centred hole, sized from the command line, written as 3MF:

```lua
-- A plate with a hole, sized from the command line
local w, h = tonumber(arg[1]) or 40, tonumber(arg[2]) or 30
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = w, height = h}
local pad = pc.part.pad{sketch = s, length = 4}
local body = pc.doc.feature{id = pad}.body
local hole = pc.sketch.new{body = body, plane = "XY", offset = 4}
pc.sketch.circle{sketch = hole, x = w / 2, y = h / 2, radius = 3}
pc.part.pocket{sketch = hole, through_all = true}
pc.doc.rebuild()
print(pc.doc.measure{body = body}.volume)
pc.file.export{path = "plate.3mf"}
```

A constrained sketch:

```lua
local s = pc.sketch.new{}
local line = pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 30, y2 = 2}
pc.sketch.constrain{sketch = s, kind = "horizontal", items = {line}}
pc.sketch.constrain{sketch = s, kind = "dimension", items = {line}, value = 25}
print(show(pc.sketch.status{sketch = s}))
```

One body on another:

```lua
local top = function(body)
  for _, f in ipairs(pc.doc.faces{body = body}) do
    if f.normal and f.normal[3] > 0.99 then return f end
  end
end
local bottom = function(body)
  for _, f in ipairs(pc.doc.faces{body = body}) do
    if f.normal and f.normal[3] < -0.99 then return f end
  end
end
pc.asm.mate{body = lid, face = bottom(lid), other = box, other_face = top(box)}
```

## Commands

<!-- commands: generated from the registered commands -->
### doc

`pc.doc.info`: The document's name, file and unit.

- Returns {name, file, unit, modified}

`pc.doc.bodies`: List the bodies.

- Returns a list of {id, name, visible, features}

`pc.doc.features`: List the features in build order.

- `body` (id, optional): Only this body's
- Returns a list of {id, name, kind, body, visible, suppressed, error}

`pc.doc.feature`: A feature with its fields.

- `id` (id)
- Returns {id, name, kind, body, visible, fields}

`pc.doc.selection`: What is selected.

- Returns {item, body, feature}, each an id or nil

`pc.doc.select`: Select a body or a feature, as a click on its row.

- `id` (id)

`pc.doc.new_body`: Make an empty body.

- `name` (string, optional)
- Returns the body's id

`pc.doc.rename`: Rename a body or a feature.

- `id` (id)
- `name` (string)

`pc.doc.set_visible`: Show or hide a body or a feature.

- `id` (id)
- `visible` (boolean)

`pc.doc.delete`: Delete a body or a feature.

- `id` (id)

`pc.doc.repair`: Repair the shapes the kernel's checker calls broken.

- `bodies` (list): The bodies
- Returns nothing; pc.doc.rebuild() waits for the repair

`pc.doc.convert_to_solid`: Turn mesh bodies into solids.

- `bodies` (list): The mesh bodies
- Returns nothing; pc.doc.rebuild() waits for the conversion

`pc.doc.suppress`: Leave a feature out of its body's solid, or back in.

- `id` (id): The feature
- `suppressed` (boolean, optional): true (the default) or false

`pc.doc.move`: Move a feature one step in its body's history.

- `id` (id): The feature
- `up` (boolean): true: earlier, false: later
- Returns whether it moved: not at the end of the history, nor past a feature it needs

`pc.doc.set_tip`: Build a body only up to a feature, or all of it again.

- `id` (id): A feature of the body
- `clear` (boolean, optional): true: build the whole history again

`pc.doc.rebuild`: Rebuild every solid that changed, repair or convert what was asked, and wait.

- `timeout` (number, optional): Seconds to wait at most (60)
- Returns a list of {feature, error} for every feature that failed

`pc.doc.faces`: The faces of a body's solid, where it sits.

- `body` (id)
- Returns a list of {index, kind, point, area, normal?, axis?, radius?}: point lies on the face, normal is a flat face's outward one, axis a turned face's {point, direction}

`pc.doc.measure`: A body's volume, surface area, centre and bounds.

- `body` (id)
- Returns {volume, area, centre, min, max, approximate}

`pc.doc.parameters`: A feature's numbers that formulas set and read.

- `id` (id): The feature
- Returns a list of {name, key, label, kind, value, text, formula, error}: name is what formulas call it (nil when they cannot), value in mm or degrees

`pc.doc.set_formula`: Set one of a feature's numbers by a formula, or take the formula away.

- `id` (id): The feature
- `parameter` (string): Its name or key, as doc.parameters lists them
- `formula` (string, optional): Such as "Printer.wall * 2"; nil takes it away
- Returns {value, text, error}: what it comes to

### var

`pc.var.new`: Make a variable set.

- `name` (string): What formulas call it: Printer.nozzle
- Returns the set's id

`pc.var.set`: Set a variable to a formula, adding it when new.

- `set` (string): The set, by name or id
- `name` (string)
- `formula` (string): Such as "0.4 mm" or "3 * Printer.nozzle"
- `comment` (string, optional)
- Returns {value, text, error}: what it comes to

`pc.var.remove`: Take a variable out of its set.

- `set` (string): The set, by name or id
- `name` (string)

`pc.var.rename`: Rename a variable, and every formula that reads it.

- `set` (string): The set, by name or id
- `name` (string)
- `to` (string)

`pc.var.list`: The variable sets and what each variable comes to.

- `set` (string, optional): Only this set, by name or id
- Returns a list of {id, name, variables}, each variable {name, formula, value, text, kind, error, comment}

`pc.var.eval`: What a formula comes to in this document.

- `formula` (string)
- Returns {value, kind, text}: value in mm or degrees

### app

`pc.app.workbenches`: The workbenches, in the order they load.

- Returns a list of {id, label, active}

`pc.app.workbench`: Switch to a workbench.

- `id` (string): As app.workbenches lists it

`pc.app.tool`: Start a toolbar tool, as a click on it does.

- `id` (string): The tool's id, as Preferences › Keyboard lists it

`pc.app.tools`: The tools of a workbench.

- `workbench` (string, optional): The active one when left out
- Returns a list of {id, label, keys}

`pc.app.quit`: Quit.

`pc.app.log`: Log panel.

### file

`pc.file.new`: New.

`pc.file.open`: Open.

- `path` (string, optional): The document to open; the dialog when left out
- Returns nothing; the document opens in its tab after the script

`pc.file.save`: Save.

`pc.file.save_as`: Save as.

- `path` (string, optional): Where to save; the dialog when left out

`pc.file.import`: Import.

- `path` (string, optional): The STEP, IGES, STL, OBJ or 3MF file; the dialog when left out
- Returns nothing; pc.doc.rebuild() waits for the import

`pc.file.export`: Export.

- `path` (string, optional): Where to write; the dialog when left out
- `format` (string, optional): step, stl or 3mf; from the path's extension when left out
- `bodies` (list, optional): The bodies to write; every visible one when left out
- `tolerance` (number, optional): The mesh formats' distance to the true surface, mm (0.01)
- Returns {path, written, skipped, triangles}

`pc.file.send_to_slicer`: Send to slicer.

### edit

`pc.edit.undo`: Undo.

`pc.edit.redo`: Redo.

`pc.edit.cut`: Cut.

`pc.edit.copy`: Copy.

`pc.edit.paste`: Paste.

`pc.edit.recompute`: Recompute all.

### view

`pc.view.fit_all`: Fit all.

`pc.view.fit_selection`: Fit selection.

`pc.view.isometric`: Isometric view.

`pc.view.front`: Front view.

`pc.view.top`: Top view.

`pc.view.right`: Right view.

`pc.view.rear`: Rear view.

`pc.view.bottom`: Bottom view.

`pc.view.left`: Left view.

`pc.view.orthographic`: Orthographic.

`pc.view.perspective`: Perspective.

`pc.view.shaded_edges`: Shaded with edges.

`pc.view.shaded`: Shaded.

`pc.view.wireframe`: Wireframe.

`pc.view.clipping_plane`: Clipping plane.

`pc.view.measure`: Measure.

`pc.view.print_bed`: Print bed.

### tab

`pc.tab.new`: New tab.

`pc.tab.close`: Close tab.

`pc.tab.next`: Next tab.

`pc.tab.previous`: Previous tab.

### sketch

`pc.sketch.new`: Make an empty sketch on a base plane.

- `body` (id, optional): The body it belongs to; the selected body, else a new one
- `plane` (string, optional): XY (the default), XZ or YZ
- `offset` (number, optional): How far along the plane's normal it sits
- `name` (string, optional): Its name in the tree
- `on` (id, optional): A datum plane, or a coordinate system whose XY, XZ or YZ plane (see plane) it takes
- `normal` (list, optional): A plane of its own instead: its normal as {x, y, z}
- `origin` (list, optional): With normal: where the plane's origin sits, {x, y, z}
- `x_axis` (list, optional): With normal: the sketch's X direction, {x, y, z}
- Returns the sketch's id

`pc.sketch.point`: Add a point.

- `sketch` (id): The sketch to draw in
- `x` (number)
- `y` (number)
- Returns the point's id

`pc.sketch.line`: Add a line from (x1, y1) to (x2, y2).

- `sketch` (id): The sketch to draw in
- `x1` (number)
- `y1` (number)
- `x2` (number)
- `y2` (number)
- Returns the line's id

`pc.sketch.polyline`: Add lines through a list of points, each ending where the next starts.

- `sketch` (id): The sketch to draw in
- `points` (list): Points as {x, y} pairs
- `closed` (boolean, optional): Join the last point to the first
- Returns the lines' ids

`pc.sketch.rect`: Add a rectangle from its corner (x, y), its width and its height.

- `sketch` (id): The sketch to draw in
- `x` (number)
- `y` (number)
- `width` (number)
- `height` (number)
- Returns the four lines' ids

`pc.sketch.circle`: Add a circle.

- `sketch` (id): The sketch to draw in
- `x` (number): The centre
- `y` (number): The centre
- `radius` (number)
- Returns the circle's id

`pc.sketch.arc`: Add an arc, counter-clockwise from the start angle to the end angle.

- `sketch` (id): The sketch to draw in
- `x` (number): The centre
- `y` (number): The centre
- `radius` (number)
- `start` (number): Degrees from the sketch's X axis
- `end` (number): Degrees from the sketch's X axis
- Returns the arc's id

`pc.sketch.geometry`: List the sketch's elements with their points.

- `sketch` (id): The sketch to draw in
- Returns a list of {id, kind, points, radius?, construction}

`pc.sketch.constrain`: Constrain elements, as the constraint's toolbar button does for a selection.

- `sketch` (id): The sketch to draw in
- `kind` (string): coincident, point_on_object, midpoint, horizontal, vertical, parallel, perpendicular, tangent, equal, symmetric, block, lock, dimension, distance, distance_x, distance_y, radius, diameter, angle, angle_x or angle_y
- `items` (list): Element ids, or "origin", "x_axis" and "y_axis"
- `value` (number, optional): A dimension's value (mm, or degrees for an angle); the measured one when left out
- Returns the new constraints' ids

`pc.sketch.set_value`: Change a dimension's value.

- `sketch` (id): The sketch to draw in
- `constraint` (id)
- `value` (number): mm, or degrees for an angle
- `driving` (boolean, optional): false makes it a reference dimension that only measures

`pc.sketch.draw`: Run a drawing or editing tool over points of the sketch, as clicks there would.

- `sketch` (id): The sketch to draw in
- `tool` (string): line, polyline, rect, rect_center, rect_rounded, circle, circle3, arc, arc3, ellipse, ellipse3, ellipse_arc, bspline, polygon, slot, arc_slot, point, fillet, chamfer, trim, extend, split, offset, translate, rotate, scale or mirror
- `points` (list): The clicks, each {x, y}, or {x = , y = , typed = {length = 20}, constrain = true} with values typed at it; "arc" and "line" switch a polyline, "finish" ends a spline
- `tolerance` (number, optional): How close a click snaps onto points and curves, mm (0.001)
- `params` (any, optional): Tool settings: polygon_sides, slot_width, fillet_radius, chamfer_length, offset_distance, copies, bspline_periodic, auto_constraints, array_rows, array_cols, array_dx, array_dy
- `construction` (boolean, optional): What it makes is construction geometry
- `avoid_redundant` (boolean, optional): Drop auto constraints that add nothing (true)
- `selection` (list, optional): The elements offset, translate, rotate, scale and mirror act on
- Returns {elements, constraints}: what it made

`pc.sketch.drag`: Drag elements by a step, the rest of the sketch following its constraints.

- `sketch` (id): The sketch to draw in
- `items` (list): The elements to drag
- `by` (list): The step, {x, y}

`pc.sketch.set_plane`: Move the sketch onto another plane, its geometry kept in its own coordinates.

- `sketch` (id): The sketch to draw in
- `normal` (list): The plane's normal, {x, y, z}
- `origin` (list, optional): Its origin, {x, y, z}
- `x_axis` (list, optional): The sketch's X direction, {x, y, z}

`pc.sketch.array`: Repeat elements in rows and columns.

- `sketch` (id): The sketch to draw in
- `items` (list): The elements to repeat
- `rows` (integer)
- `cols` (integer)
- `dx` (number): The step between columns, mm
- `dy` (number): The step between rows, mm
- Returns {elements}: what it made

`pc.sketch.set_constraint`: Make constraints driving or reference, active or not.

- `sketch` (id): The sketch to draw in
- `items` (list): The constraints
- `driving` (boolean, optional): false: a reference dimension that only measures
- `active` (boolean, optional): false: kept but not solved

`pc.sketch.mirror_sketch`: A new sketch on the same plane: this one's geometry mirrored across its Y axis.

- `sketch` (id): The sketch to draw in
- Returns the new sketch's id

`pc.sketch.merge`: A new sketch holding this one's geometry and other sketches', mapped onto its plane.

- `sketch` (id): The sketch to draw in
- `with` (list): The other sketches
- Returns the new sketch's id

`pc.sketch.carbon_copy`: Copy another sketch's geometry into this one, mapped onto its plane.

- `sketch` (id): The sketch to draw in
- `from` (id): The sketch to copy
- Returns {elements, constraints}: what it made

`pc.sketch.paste`: Add geometry held as a sketch of its own, moved by a step.

- `sketch` (id): The sketch to draw in
- `clipboard` (any): The geometry, as a sketch's fields (what copying in the sketcher holds)
- `by` (list): The step, {x, y}
- Returns {elements}: what it made

`pc.sketch.external`: Project edges of solids into the sketch as fixed references.

- `sketch` (id): The sketch to draw in
- `edges` (list): Each {body, point, direction}: a point on the edge and its direction, in the body's own frame
- Returns {elements}: what it made

`pc.sketch.constraints`: List the sketch's constraints.

- `sketch` (id): The sketch to draw in
- Returns a list of {id, kind, items, value?}

`pc.sketch.status`: How constrained the sketch is, and what conflicts.

- `sketch` (id): The sketch to draw in
- Returns {dof, solved, redundant, conflicting}

`pc.sketch.delete`: Delete elements or constraints, and what depends on them.

- `sketch` (id): The sketch to draw in
- `items` (list): Element or constraint ids

`pc.sketch.construction`: Make elements construction geometry, or normal again.

- `sketch` (id): The sketch to draw in
- `items` (list): Element ids
- `on` (boolean, optional): true (the default) or false

### part

`pc.part.pad`: Pad a sketch.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.pocket`: Cut a sketch into the body.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.revolve`: Turn a sketch about an axis.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.groove`: Cut a sketch turned about an axis.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.loft`: Loft through sketches.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.subtractive_loft`: Cut a loft through sketches.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.pipe`: Sweep a sketch along a path.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.subtractive_pipe`: Cut a sketch swept along a path.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.helix`: Sweep a sketch along a helix.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.subtractive_helix`: Cut a sketch swept along a helix.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.primitive`: Add a box, cylinder, sphere, cone, torus or wedge.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `variant` (string, optional): box (the default), cylinder, sphere, cone, torus or wedge
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.subtractive_primitive`: Cut a box, cylinder, sphere, cone, torus or wedge.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `variant` (string, optional): box (the default), cylinder, sphere, cone, torus or wedge
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.hole`: Drill holes at a sketch's circles and points.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.fillet`: Round edges.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.chamfer`: Bevel edges.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.draft`: Tilt faces.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.thickness`: Hollow the solid.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.mirror`: Mirror the last feature.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.linear_pattern`: Repeat the last feature along a line.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.polar_pattern`: Repeat the last feature about an axis.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.scaled`: Scale the last feature.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.boolean`: Combine with another body.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

`pc.part.set`: Change fields of a Part Design feature or a datum.

- `feature` (id): The feature to change
- Other arguments: The fields to change, such as length = 25

`pc.part.datum`: Add a datum plane, line, point or coordinate system.

- `kind` (string): plane, line, point or coordinate_system
- `body` (id): The body it belongs to
- `plane` (string, optional): The base plane it sits on: XY (the default), XZ or YZ
- `face_point` (list, optional): Or a flat face it sits on: a point of the face, {x, y, z}
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- `offset` (list, optional): Moved along its own x, y and normal, {x, y, z} in mm
- `rotation` (number, optional): Turned about its normal, degrees
- `flip` (boolean, optional): Turned to face the other way
- `size` (number, optional): How large it draws, mm
- `name` (string, optional): Its name in the tree
- Returns the datum's id

### asm

`pc.asm.mate`: Put two flat faces against each other.

- `body` (id): The body that moves
- `face` (any): A flat face, {point, normal}, as pc.doc.faces lists it
- `other` (id): The body it is held against
- `other_face` (any): A flat face, {point, normal}, as pc.doc.faces lists it
- `name` (string, optional): Its name in the tree
- `offset` (number, optional): The gap between them, mm
- `flip` (boolean, optional): Face the same way instead of at each other
- Returns the joint's id

`pc.asm.align`: Put two round faces on one axis.

- `body` (id): The body that moves
- `face` (any): A round face, {axis = {point, direction}}, as pc.doc.faces lists it
- `other` (id): The body it is held against
- `other_face` (any): A round face, {axis = {point, direction}}, as pc.doc.faces lists it
- `name` (string, optional): Its name in the tree
- Returns the joint's id

`pc.asm.angle`: Hold two flat faces at an angle.

- `body` (id): The body that moves
- `face` (any): A flat face, {point, normal}, as pc.doc.faces lists it
- `other` (id): The body it is held against
- `other_face` (any): A flat face, {point, normal}, as pc.doc.faces lists it
- `name` (string, optional): Its name in the tree
- `degrees` (number, optional): Between their outward normals; the angle they make now when left out
- Returns the joint's id

`pc.asm.set`: Change a joint's gap, side or angle.

- `joint` (id)
- `offset` (number, optional): A mate's gap, mm
- `flip` (boolean, optional): A mate's side
- `degrees` (number, optional): An angle joint's angle

`pc.asm.solve`: Place every body its joints hold.

- Returns what moved, in words

`pc.asm.placement`: Where a body sits.

- `body` (id)
- Returns {translation, rotation}, rotation a quaternion {x, y, z, w}

`pc.asm.place`: Put a body at a placement.

- `body` (id)
- `translation` (list, optional): {x, y, z} in mm
- `rotation` (list, optional): A quaternion {x, y, z, w}

`pc.asm.move`: Move a body by a step and a turn.

- `body` (id)
- `by` (list, optional): {x, y, z} in mm
- `turn` (number, optional): Degrees about `axis`
- `axis` (list, optional): {x, y, z}; Z when left out
- `about` (list, optional): The point the turn is about, {x, y, z}; the origin when left out
<!-- /commands -->
