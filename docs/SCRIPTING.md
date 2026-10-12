# Scripting

printCAD runs Lua 5.4 scripts. Everything the application and its
workbenches can do by command is a function under `pc`, called with named
arguments in braces:

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 30, height = 20}
local pad = pc.design.pad{sketch = s, length = 12}
pc.design.set{feature = pad, length = 20}
```

The Design workbench's commands also answer to `part` (`pc.part.pad`,
`part.set`), so older scripts, recordings and key settings keep working.

## Where scripts run

- **The console.** Windows › Console, Scripts › Console, or the toolbar's
  Scripts button. Enter runs what is typed, Shift+Enter starts another
  line, Tab completes a command name, Up and Down go back through what was
  run, earlier sessions included. An expression shows its value. Globals stay set between runs, and
  `local` names last one run only. Save as script writes everything run
  in the console since the start as a new script in the scripts folder.
- **Script files.** Scripts › Run script… runs any `.lua` file. Every
  `.lua` file in the scripts folder (`~/.config/printcad/scripts` on
  Linux; Scripts › Open scripts folder opens it) is also a command of its
  own: it shows in the Scripts menu, the toolbar's Scripts
  button and the command palette, and takes a key in Preferences ›
  Keyboard. Its first comment line is its description. Scripts › New
  script starts one from a template.
- **The command line.** No window opens:

  ```sh
  printcad --script build.lua --open part.prtcad --save out.prtcad -- 40 20
  ```

  `--open` and `--save` are optional. The words after `--` are the
  script's `arg` table. Output goes to stdout and log lines to stderr. The
  exit code is 0 when the script finishes, 1 when it stops on an error and
  2 when the command line is wrong. Commands that need a window (the view,
  the selection, tools) are not available there; `pc.file.save_as` and
  `pc.file.export` take a `path`.

## Working with commands

- `help()` lists every command; `help("sketch")` those starting with
  `sketch`. `show(value)` gives a table as readable text, for `print`;
  it prints nothing itself: `print(show(t))`.
- Ids of bodies, features, sketch elements and constraints are strings.
  Commands that make something answer its id.
- An empty `{}` in a command's arguments is the empty list. `array()`
  says the same anywhere else, and `array(1, 2)` is the same as `{1, 2}`.
- A feature's fields are what `pc.doc.feature{id = f}.fields` shows, set
  by name (`pc.design.set{feature = f, length = 25}`). A field holding no
  value is nil in Lua, so `fields` leaves it out and `unset` names it. A
  choice is its name as a string (`axis = "SketchY"`); a choice that
  carries values is a table under its name:
  `axis = {Custom = {origin = {0, 5}, dir = {0, 1}}}` turns a revolution
  about the line through (0, 5) along the sketch's Y, in sketch
  coordinates. A datum is `{Datum = id}` wherever a feature takes one (a
  revolution's axis, a mirror's plane, a pattern's direction), and an
  empty `{}` stands for a table of settings where one is wanted
  (`params = {}`).
- A command that fails raises a Lua error with the reason, which stops the
  script. `pcall(pc.design.pad, {sketch = s})` catches it instead.
- Every change is an ordinary edit, so Undo takes it back. A console line
  or a script run is one undo step, and so is anything you edit by hand
  while it runs.
- A run that stops with an error, or is stopped, takes back everything
  changed while it ran and leaves no undo step: the document is as it was
  before the run. An import is the exception; it cannot be taken back, so
  a failed run keeps it and takes back only what came after it.
  `printcad --script` reports the error and leaves the `--save` file
  unwritten.
- Solids rebuild after a script, as they do after a click. To read a solid
  in the same script, call `pc.doc.rebuild()` first: it waits and answers
  the features that failed.
- `pc.doc.faces{body = b}` lists a solid's faces with a point on each and
  a flat face's normal or a round face's axis. Assembly joints take those
  faces as they are listed.
- `pc.doc.edges{body = b}` lists a solid's edges: line, circle or other,
  a point halfway along each and its direction there, its length, the
  indices `doc.faces` gives the two faces it runs between and their names
  (strings), and a circle's centre and radius. A fillet, a chamfer or a
  surface step takes an edge as that point and direction:

  ```lua
  local pick
  for _, e in ipairs(pc.doc.edges{body = body}) do
    if e.kind == "line" and math.abs(e.direction[3]) > 0.99 then pick = e end
  end
  pc.design.fillet{body = body, radius = 2, edges = {Edges = {
    {point = pick.point, direction = pick.direction, faces = pick.names}}}}
  ```

  Points and directions are where the body sits; for a body with a
  placement of its own, a feature's picks are in the body's frame.
- A script's `return` is its answer: the console shows it, the command
  line prints it after what the script printed, and an AI agent's `lua`
  gets it back as JSON (tables as objects or lists).
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
- A Design feature records when its task closes with OK, as the
  command that makes it with the fields that differ from what that command
  makes on its own. An edit records as `design.set` with the fields changed.
  A new Surface step records as the command that makes it, with all its
  fields.
- A joint records with its faces where the bodies were before it moved
  them; a move records the placement it ended at.
- The sketcher's other actions record too: arrays, cut and paste (the
  pasted geometry goes into the script), mirrored and merged sketches,
  carbon copies, external geometry, a new plane, driving and active flags.
- Renaming, showing or hiding, suppressing, reordering, moving the tip of
  and deleting tree rows record as `doc.*`, as do Repair shape, Convert
  to solid and Refine shape; variables and formulas as `var.*`,
  `doc.set_formula` and `doc.set_value`; an import records as
  `file.import` with its path; the Solve button as `asm.solve`.
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
local pad = pc.design.pad{sketch = s, length = 5}
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
local pad = pc.design.pad{sketch = s, length = 4}
local body = pc.doc.feature{id = pad}.body
local hole = pc.sketch.new{body = body, plane = "XY", offset = 4}
pc.sketch.circle{sketch = hole, x = w / 2, y = h / 2, radius = 3}
pc.design.pocket{sketch = hole, through_all = true}
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

Longer workflows, each a script that checks what it made, are under
[recipes](recipes/).

## Commands

Some commands carry notes (what they refuse, do without saying, or are
easily mistaken for) and an example. The test suite runs every example
of every command, and every recipe, from an empty document, so they
work as written.

<!-- commands: generated from the registered commands -->
### doc

`pc.doc.info`: The document's name, file and unit.

- Returns {name, file, unit, modified}

Notes:

- `file` is nil until the document is saved, and `modified` turns true with the first edit. `unit` is the unit numbers are shown in ("mm"); commands take and give lengths in millimetres whatever it is.

Example: A new document, edited.

```lua
local info = pc.doc.info()
assert(info.unit == "mm" and info.file == nil and not info.modified)
pc.sketch.new{plane = "XY"}
assert(pc.doc.info().modified, "an edit marks it modified")
```

`pc.doc.agent_rules`: The rules an AI agent working on this document keeps to, beside the ones in Preferences for every document.

- Returns the rules, as text

Notes:

- It needs the app's window: a `printcad --script` run refuses it.

See also `pc.doc.set_agent_rules`.

`pc.doc.set_agent_rules`: Set the rules an AI agent working on this document keeps to; they are saved with the document.

- `rules` (string): The rules, as plain text

Notes:

- The text replaces the rules the document had; an empty text takes them away. The rules in Preferences, for every document, are kept apart and stay.
- It needs the app's window: a `printcad --script` run refuses it.

See also `pc.doc.agent_rules`.

`pc.doc.bodies`: List the bodies.

- Returns a list of {id, name, visible, frozen, selectable, material, features}

Notes:

- `features` are the body's feature ids in build order; `material` is nil until `pc.doc.set_body` gives it one.
- A sketch made without a `body` starts a body of its own, so one sketch and its pad are one body.

See also `pc.doc.features`, `pc.doc.set_body`.

Example: A padded block's one body.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
pc.doc.set_body{body = body, material = {name = "PLA", density = 1.24}}
local bodies = pc.doc.bodies()
assert(#bodies == 1 and bodies[1].id == body and bodies[1].visible)
assert(bodies[1].features[1] == s and bodies[1].features[2] == pad)
assert(bodies[1].material.name == "PLA")
assert(math.abs(bodies[1].material.density - 1.24) < 1e-6)
```

`pc.doc.features`: List the features in build order.

- `body` (id, optional): Only this body's
- Returns a list of {id, name, kind, body, visible, suppressed, error}

Notes:

- `error` is set only on a feature the last `pc.doc.rebuild()` could not build. Variable sets and the configurations table are listed too, with no body.
- A `body` that is not a body of the document gives an empty list, not an error.

See also `pc.doc.feature`, `pc.doc.bodies`.

Example: A pad and its sketch, in order.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
local list = pc.doc.features{body = body}
assert(#list == 2)
assert(list[1].id == s and list[1].kind == "Sketch")
assert(list[2].id == pad and list[2].kind == "Pad" and list[2].error == nil)
assert(not list[1].visible, "a pad hides its sketch")
```

`pc.doc.feature`: A feature with its fields.

- `id` (id)
- Returns {id, name, kind, body, visible, suppressed, error, fields, unset, values}: unset names the fields holding no value, which fields leaves out; values is the data as the feature is built now (formulas worked out, a plane following what it stands on), given when it differs from fields

Notes:

- `fields` keep the plain numbers a formula stands over: read `values` for what the feature is built from. A body's id is refused ("is not a feature of this document").

See also `pc.doc.parameters`, `pc.doc.features`.

Example: A pad's length, kept and worked out.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local f = pc.doc.feature{id = pad}
assert(f.kind == "Pad" and f.fields.Pad.length == 5 and f.fields.Pad.sketch == s)
assert(f.values == nil, "no formula: values is fields")
pc.doc.set_formula{id = pad, parameter = "length", formula = "2 * 4 mm"}
f = pc.doc.feature{id = pad}
assert(f.fields.Pad.length == 5 and f.values.Pad.length == 8)
```

`pc.doc.selection`: What is selected.

- Returns {item, body, feature}, each an id or nil

Notes:

- `item` is the tree row selected, `body` the body being worked on and `feature` the feature being worked on. It needs the app's window: a `printcad --script` run refuses it.

See also `pc.doc.select`.

`pc.doc.select`: Select a body or a feature, as a click on its row.

- `id` (id)

Notes:

- It takes a body, a feature, an imported part or a component; any other id is refused ("is not in this document"). Selecting a feature moves its body's tip to it, as a click on its row does.
- It needs the app's window: a `printcad --script` run refuses it.

See also `pc.doc.selection`, `pc.doc.set_tip`.

`pc.doc.new_body`: Make an empty body.

- `name` (string, optional)
- Returns the body's id

Notes:

- Without a name it is called `body`, `body_1` and so on. Features go into it only when given its id as `body`: `pc.sketch.new` without one starts a body of its own.
- It has no solid until a feature in it builds: `pc.doc.measure` refuses it ("the body has no solid yet").

See also `pc.doc.bodies`.

Example: A named body with a pad in it.

```lua
local lid = pc.doc.new_body{name = "Lid"}
local s = pc.sketch.new{plane = "XY", body = lid}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 2}
assert(pc.doc.feature{id = pad}.body == lid)
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = lid}.volume - 400) < 1e-6)
assert(pc.doc.bodies()[1].name == "Lid")
```

`pc.doc.rename`: Rename a body or a feature.

- `id` (id)
- `name` (string)

Notes:

- Every formula that reads a renamed feature is rewritten to the new name. Two features may share a name; an empty name leaves the name as it was.
- A variable set is a feature: this renames it too.

See also `pc.var.rename`.

Example: A formula follows a renamed pad.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
pc.doc.set_formula{id = pad, parameter = "length2", formula = "Pad.length * 2"}
pc.doc.rename{id = pad, name = "Base"}
assert(pc.doc.feature{id = pad}.name == "Base")
assert(pc.doc.parameters{id = pad}[2].formula == "Base.length * 2")
pc.doc.rename{id = pc.doc.feature{id = pad}.body, name = "Bracket"}
assert(pc.doc.bodies()[1].name == "Bracket")
```

`pc.doc.set_body`: Change a body: its colour, how much shows through, material, placement, whether it is frozen or clicks pick it.

- `body` (id)
- `color` (any, optional): {r, g, b} from 0 to 1, or nil for the colour it came with
- `opacity` (number, optional): 1 solid, less to see through
- `material` (any, optional): {name, density} with the density in g/cm³, or nil for none
- `frozen` (boolean, optional): Keep it as it stands: its features are not rebuilt until it thaws
- `selectable` (boolean, optional): false lets clicks pass through it
- `face_colors` (list, optional): An empty list gives every face the body's colour again
- `translation` (any, optional): {x, y, z}: where its origin goes; bodies moving as one with it follow
- `rotation` (any, optional): {x, y, z, w}: its turn as a quaternion

Notes:

- Only what is given changes. `translation` and `rotation` set the placement outright, not added to the one it has; the turn is about the body's own origin, and either may be a list ({100, 0, 0}) as well as named parts.
- Colour parts are kept between 0 and 1 and `opacity` between 0.05 and 1. A `material` without a density above 0 is refused; its name defaults to "Material".
- While `frozen`, edits to its features build nothing and `pc.doc.rebuild()` reports nothing for it; thawing builds what changed meanwhile.

See also `pc.doc.set_face_color`, `pc.doc.measure`.

Example: A block moved and turned.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
assert(#pc.doc.rebuild() == 0)
pc.doc.set_body{body = body, translation = {x = 100, y = 0, z = 0}, color = {r = 0.9, g = 0.4, b = 0.1}}
local m = pc.doc.measure{body = body}
assert(math.abs(m.min[1] - 100) < 1e-4 and math.abs(m.max[1] - 120) < 1e-4)
-- A quarter turn about Z, about the body's origin.
local h = math.sqrt(0.5)
pc.doc.set_body{body = body, rotation = {x = 0, y = 0, z = h, w = h}}
m = pc.doc.measure{body = body}
assert(math.abs(m.min[1] - 90) < 1e-4 and math.abs(m.max[2] - 20) < 1e-4)
assert(math.abs(m.volume - 1000) < 1e-6)
```

`pc.doc.set_textures`: Set every surface texture pressed into a body's faces for printing: drawn in the view, baked into STL and 3MF files and what goes to the slicer.

- `body` (id)
- `textures` (list): Each {texture = {pattern, projection, tile_mm, depth_mm, rotation_deg, inward, keep_flat_deg}, faces = {...}}: pattern Knurl, Ribs, Dots, Hex, Bricks, Waves, Noise or Crosshatch; projection Triplanar, {Planar = "Z"}, {Cylindrical = "Z"} or Spherical; faces as doc.faces numbers them, none for every face; an empty list takes them all away

Notes:

- Each call replaces every texture the body had. A texture needs `pattern`, `projection`, `tile_mm` and `depth_mm`; `rotation_deg`, `inward` and `keep_flat_deg` default to 0 and false.
- The solid keeps its shape: `pc.doc.measure` and `pc.doc.faces` read it untextured. The texture is pressed into the mesh STL and 3MF exports write.
- Faces given by number need the body built ("the body has no solid yet"); a texture with no faces needs none.

See also `pc.doc.faces`, `pc.file.export`.

Example: Ribs on the top face, in the exported mesh.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 6, height = 6}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
assert(#pc.doc.rebuild() == 0)
local top
for _, face in ipairs(pc.doc.faces{body = body}) do
  if face.kind == "plane" and face.normal[3] > 0.99 then top = face.index end
end
local path = os.tmpname()
local function triangles()
  local out = pc.file.export{path = path, format = "stl"}
  os.remove(out.path)
  return out.triangles
end
local plain = triangles()
pc.doc.set_textures{body = body, textures = {{
  texture = {pattern = "Ribs", projection = {Planar = "Z"}, tile_mm = 3, depth_mm = 0.5},
  faces = {top},
}}}
assert(triangles() > 100 * plain, "the ribs are pressed into the exported mesh")
assert(math.abs(pc.doc.measure{body = body}.volume - 180) < 1e-6, "the solid keeps its shape")
pc.doc.set_textures{body = body, textures = {}}
assert(triangles() == plain)
os.remove(path)
```

`pc.doc.set_face_color`: Colour one face of a body, over the body's colour.

- `body` (id)
- `face` (integer): The face, as doc.faces numbers it from 0
- `color` (any, optional): {r, g, b} from 0 to 1, or nil for the body's colour

Notes:

- It needs the body built: before `pc.doc.rebuild()` it is refused ("the body has no solid yet"), and a face number past the last is refused too.
- The colour can be a list ({1, 0, 0}) as well as named parts. `pc.doc.set_body{body = ..., face_colors = {}}` takes every face's colour away at once.

See also `pc.doc.faces`, `pc.doc.set_body`.

Example: A pad's top painted red in a picture, then the body's colour again.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
assert(#pc.doc.rebuild() == 0)
local top
for _, face in ipairs(pc.doc.faces{body = body}) do
  if face.kind == "plane" and face.normal[3] > 0.99 then top = face.index end
end
local path = os.tmpname()
local function picture()
  pc.doc.picture{path = path, view = "top", size = {64, 64}}
  local file = io.open(path, "rb")
  local bytes = file:read("a")
  file:close()
  return bytes
end
local plain = picture()
pc.doc.set_face_color{body = body, face = top, color = {r = 1, g = 0, b = 0}}
local red = picture()
pc.doc.set_face_color{body = body, face = top}
assert(red ~= plain and picture() == plain, "red, then the body's colour again")
os.remove(path)
```

`pc.doc.linked_copy`: A linked copy of a body beside it: the same shape, following every change.

- `body` (id)
- Returns the copy's id

Notes:

- The copy stands 10 mm clear of the original along X, placed by its own placement (`pc.doc.set_body`). It follows the original's changes once they are built, and takes no features of its own.

See also `pc.doc.set_body`.

Example: A copy that follows its original.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
assert(#pc.doc.rebuild() == 0)
local copy = pc.doc.linked_copy{body = body}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = copy}
assert(math.abs(m.volume - 1000) < 1e-6)
assert(math.abs(m.min[1] - 30) < 1e-4, "10 mm clear of the original along X")
pc.doc.set_value{id = pad, parameter = "length", value = 8}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = copy}.volume - 1600) < 1e-6, "it follows")
```

`pc.doc.move_after`: Move a feature in its body's history to just after another.

- `id` (id): The feature
- `after` (id, optional): The feature it goes after; first in its body when left out

Notes:

- It refuses to carry a feature past one it is built from or one built from it ("depend on each other and keep their order"), and two features of different bodies ("not in one body"). The whole move is checked before anything moves: a refused one leaves the history as it was. Moving a feature after itself does nothing.
- Whether the feature still builds in its new place is told by `pc.doc.rebuild()`, not here.

See also `pc.doc.move`.

Example: A sketch brought forward, its pocket kept after the pad.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
local top = pc.sketch.new{plane = "XY", offset = 5, body = body}
pc.sketch.circle{sketch = top, x = 10, y = 5, radius = 2}
local hole = pc.design.pocket{sketch = top, depth = 3}
local ok, why = pcall(pc.doc.move_after, {id = hole, after = s})
assert(not ok and tostring(why):find("depend on each other"), tostring(why))
pc.doc.move_after{id = top, after = s}
local order = pc.doc.features{body = body}
assert(order[1].id == s and order[2].id == top and order[3].id == pad and order[4].id == hole)
assert(#pc.doc.rebuild() == 0)
```

`pc.doc.recompute`: Build a body again from its history.

- `body` (id)

Notes:

- It builds nothing itself: it marks every feature of the body for building from the start, and `pc.doc.rebuild()` builds them and waits. Edits mark what they change already, so a script needs it only to build a body afresh.
- A feature's id is refused ("is not a body of this document").
- A frozen body is refused ("is frozen"): it keeps the solid it has. Thawed with `pc.doc.set_body{body = ..., frozen = false}`, it builds what changed meanwhile.

See also `pc.doc.rebuild`, `pc.doc.set_body`.

Example: A body built again from its sketch.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
assert(#pc.doc.rebuild() == 0)
pc.doc.recompute{body = body}
assert(#pc.doc.rebuild() == 0, "built again from the sketch")
assert(math.abs(pc.doc.measure{body = body}.volume - 1000) < 1e-6)
pc.doc.set_body{body = body, frozen = true}
local ok, why = pcall(pc.doc.recompute, {body = body})
assert(not ok and tostring(why):find("is frozen"), tostring(why))
assert(math.abs(pc.doc.measure{body = body}.volume - 1000) < 1e-6, "kept")
```

`pc.doc.set_visible`: Show or hide a body or a feature.

- `id` (id)
- `visible` (boolean)

Notes:

- Hiding changes what is drawn and picked, not what is built: a hidden body still builds and measures, and a hidden feature stays in its body's solid (`pc.doc.suppress` takes it out).
- `pc.doc.picture` leaves hidden bodies out unless they are named in its `bodies`.

See also `pc.doc.suppress`.

Example: A sketch shown, a body hidden.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
pc.doc.set_visible{id = s, visible = true}
assert(pc.doc.feature{id = s}.visible)
pc.doc.set_visible{id = body, visible = false}
assert(not pc.doc.bodies()[1].visible)
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.volume - 1000) < 1e-6, "hidden, still built")
```

`pc.doc.delete`: Delete a body or a feature.

- `id` (id)

Notes:

- A body goes with every feature in it.
- A feature goes alone: deleting a pad keeps its sketch, and deleting a sketch a later feature uses is not refused; that feature then fails at `pc.doc.rebuild()` ("references a missing sketch").

See also `pc.doc.suppress`.

Example: A fillet taken away again.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 20}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
local round = pc.design.fillet{body = body, radius = 1}
pc.doc.delete{id = round}
assert(#pc.doc.rebuild() == 0)
assert(#pc.doc.faces{body = body} == 6, "the block is square again")
assert(#pc.doc.features{body = body} == 2, "the sketch and the pad")
```

`pc.doc.repair`: Repair the shapes the kernel's checker calls broken.

- `bodies` (list): The bodies
- Returns nothing; pc.doc.rebuild() waits for the repair

Notes:

- It asks for the repair of imported, converted and based solids; any other body, or one whose repair is already asked for, is passed over without an error. Asking clears the undo history, as an import does.
- It needs the app's window: a `printcad --script` run refuses it.

See also `pc.doc.rebuild`.

`pc.doc.convert_to_solid`: Turn mesh bodies into solids.

- `bodies` (list): The mesh bodies
- Returns nothing; pc.doc.rebuild() waits for the conversion

Notes:

- Only mesh bodies (imported STL, OBJ, 3MF and the like) are converted; any other body is passed over without an error. Asking clears the undo history, as an import does.
- Curved stretches stay facets: `pc.doc.refine` rebuilds them on true surfaces. It needs the app's window: a `printcad --script` run refuses it.

See also `pc.doc.refine`, `pc.doc.rebuild`.

`pc.doc.refine`: Rebuild converted solids' facets on the cylinders, cones, spheres and tori they approximate.

- `bodies` (list): The converted bodies
- Returns nothing; pc.doc.rebuild() waits for the refine

Notes:

- Only a converted body still made of facets, with no base shape, is refined; any other body is passed over without an error. Asking clears the undo history.
- It needs the app's window: a `printcad --script` run refuses it.

See also `pc.doc.convert_to_solid`.

`pc.doc.replace_shape`: Read a body's shape from another file: its first solid becomes the shape the body's features build on.

- `body` (id): An imported or converted body, or one with a base shape
- `path` (string): A STEP, IGES or mesh file
- Returns nothing; pc.doc.rebuild() waits for the new shape

Notes:

- A body that builds its shape from its own history, a linked copy and a part linked from another file are refused; a file that cannot be read is refused before anything changes. The file is kept in the document, and the change clears the undo history, as an import does.
- It needs the app's window: a `printcad --script` run refuses it.

See also `pc.doc.rebuild`.

`pc.doc.suppress`: Leave a feature out of its body's solid, or back in.

- `id` (id): The feature
- `suppressed` (boolean, optional): true (the default) or false

Notes:

- The feature stays in the history with its data and is built again once unsuppressed; `pc.doc.rebuild()` builds the body without it. A body's id is refused ("is not a feature of this document").

See also `pc.doc.set_visible`, `pc.doc.set_tip`, `pc.doc.delete`.

Example: A pocket left out and put back.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
local top = pc.sketch.new{plane = "XY", offset = 5, body = body}
pc.sketch.circle{sketch = top, x = 10, y = 5, radius = 2}
local hole = pc.design.pocket{sketch = top, depth = 3}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.volume - (1000 - math.pi * 4 * 3)) < 1e-3)
pc.doc.suppress{id = hole}
assert(pc.doc.feature{id = hole}.suppressed)
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.volume - 1000) < 1e-6, "the plain block")
pc.doc.suppress{id = hole, suppressed = false}
assert(#pc.doc.rebuild() == 0)
assert(pc.doc.measure{body = body}.volume < 1000, "cut again")
```

`pc.doc.move`: Move a feature one step in its body's history.

- `id` (id): The feature
- `up` (boolean): true: earlier, false: later
- Returns true; a move past the end of the history, or past a feature one of the two is built from, fails saying so

Notes:

- It changes the order the features build in; `pc.doc.set_tip` changes how far the body builds and leaves the order alone.
- Only what one feature is built from is checked: a fillet moved above the pad it rounds is let through, and `pc.doc.rebuild()` then reports it ("needs existing material").

See also `pc.doc.move_after`, `pc.doc.set_tip`.

Example: A pad stays after its sketch; a fillet moved before it fails.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
local round = pc.design.fillet{body = body, radius = 1}
local ok, why = pcall(pc.doc.move, {id = pad, up = true})
assert(not ok and tostring(why):find("built from"), tostring(why))
assert(pc.doc.move{id = round, up = true})
assert(pc.doc.features{body = body}[2].id == round)
local failed = pc.doc.rebuild()
assert(#failed == 2 and failed[1].feature == round, "a fillet before the pad has nothing to round")
```

`pc.doc.set_tip`: Build a body only up to a feature, or all of it again.

- `id` (id): A feature of the body
- `clear` (boolean, optional): true: build the whole history again

Notes:

- The features after the tip stay in the history, left out of the solid until the tip moves past them; `clear = true` takes the tip away, whichever of the body's features `id` names.
- A feature made while the tip is set goes in right after it, and the tip moves to the new feature.

See also `pc.doc.move`, `pc.doc.suppress`.

Example: A body built only up to its pad.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
local top = pc.sketch.new{plane = "XY", offset = 5, body = body}
pc.sketch.circle{sketch = top, x = 10, y = 5, radius = 2}
local hole = pc.design.pocket{sketch = top, depth = 3}
pc.doc.set_tip{id = pad}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.volume - 1000) < 1e-6, "built up to the pad")
pc.doc.set_tip{id = pad, clear = true}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.volume - (1000 - math.pi * 12)) < 1e-3)
```

`pc.doc.rebuild`: Rebuild every solid that changed, repair or convert what was asked, and wait.

- `timeout` (number, optional): Seconds to wait at most (60)
- Returns a list of {feature, error} for every feature that failed

Notes:

- Commands that make or change features build nothing; this builds them, and is where a feature that cannot build is told. An empty list means every feature built: check `#pc.doc.rebuild() == 0` before measuring.
- A body whose feature failed keeps the solid of the history before that feature, so a measurement after a failure measures the earlier solid.

See also `pc.doc.measure`, `pc.doc.faces`.

Example: A profile that does not close is told here.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.polyline{sketch = s, points = {{0, 0}, {10, 0}, {10, 10}}}
local pad = pc.design.pad{sketch = s, length = 5}
local failed = pc.doc.rebuild()
assert(#failed == 1 and failed[1].feature == pad, "the pad failed")
assert(failed[1].error:find("not closed"), failed[1].error)
```

`pc.doc.faces`: The faces of a body's solid, where it sits.

- `body` (id)
- `frame` (string, optional): world (the default): where the body sits; body: in the body's own frame, as features take faces and edges
- Returns a list of {index, kind, point, area, normal?, axis?, radius?, name?}: point lies on the face, normal is a flat face's outward one, axis a turned face's {point, direction}

Notes:

- It reads the built solid: run `pc.doc.rebuild()` first; a body not yet built is refused ("the body has no solid yet").
- Points, normals and axes are in world space, where the body sits; features take faces in the body's own frame, the same unless the body was moved: for a moved body ask with frame = "body" to pass a face to a feature.
- Every rebuild numbers the faces afresh: find a face by its kind, normal and point in the same script rather than keep its index. `name` is a string.

See also `pc.doc.measure`.

Example: A moved block's top, where it sits and as a feature takes it.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local body = pc.doc.feature{id = pc.design.pad{sketch = s, length = 4}}.body
pc.asm.place{body = body, translation = {0, 0, 50}}
assert(#pc.doc.rebuild() == 0)
local function top(faces)
  for _, face in ipairs(faces) do
    if face.kind == "plane" and face.normal[3] > 0.99 then return face end
  end
end
assert(math.abs(top(pc.doc.faces{body = body}).point[3] - 54) < 1e-6)
local own = top(pc.doc.faces{body = body, frame = "body"})
assert(math.abs(own.point[3] - 4) < 1e-6, "4 in the body's own frame")
```

`pc.doc.edges`: The edges of a body's solid, where it sits.

- `body` (id)
- `frame` (string, optional): world (the default): where the body sits; body: in the body's own frame, as features take faces and edges
- Returns a list of {index, kind, point, direction, length, faces, names?, centre?, normal?, radius?}: kind is line, circle or other; point lies halfway along the edge and direction is its way there, in world space, as an edge pick takes them ({point = e.point, direction = e.direction}); faces are the indices doc.faces gives the two faces it runs between, names theirs as strings; a circle's centre, normal and radius

Notes:

- It reads the built solid: run `pc.doc.rebuild()` first; a body not yet built is refused ("the body has no solid yet").
- `length` is measured along the drawn outline, a little under a curved edge's true length (31.40 for a 5 mm circle's 31.42). The seam of a turned face is an edge with one face in `faces`.
- Every rebuild numbers the edges afresh, as it does the faces: find an edge by its kind, point and faces in the same script.
- As doc.faces, world space unless frame = "body", which is what a fillet or chamfer of a moved body takes.

See also `pc.doc.faces`.

Example: A cylinder's rims and seam.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 5}
local pad = pc.design.pad{sketch = s, length = 10}
assert(#pc.doc.rebuild() == 0)
local body = pc.doc.feature{id = pad}.body
local rims, seams = 0, 0
for _, edge in ipairs(pc.doc.edges{body = body}) do
  if edge.kind == "circle" then
    rims = rims + 1
    assert(math.abs(edge.radius - 5) < 1e-6 and #edge.faces == 2)
    assert(math.abs(edge.length - 2 * math.pi * 5) < 0.05, "measured along the outline")
  elseif #edge.faces == 1 then
    seams = seams + 1
    assert(edge.kind == "line" and math.abs(edge.length - 10) < 1e-6)
  end
end
assert(rims == 2 and seams == 1, "the top and bottom rims and the side's seam")
```

`pc.doc.measure`: A body's volume, surface area, centre and bounds.

- `body` (id)
- Returns {volume, area, centre, min, max, approximate}

Notes:

- It reads the built solid: run `pc.doc.rebuild()` first; a body not yet built is refused ("the body has no solid yet").
- Volume is in mm³ and area in mm², the bounds and centre in world space. `approximate` is true when some face had no closed form and the figures were summed over its triangles: close (a fraction of a percent) rather than exact. A mesh body is refused; it has no solid to measure.

See also `pc.doc.rebuild`, `pc.doc.faces`.

Example: A cylinder's volume and bounds.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 5}
local pad = pc.design.pad{sketch = s, length = 10}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.volume - math.pi * 25 * 10) < 1e-3, m.volume)
assert(math.abs(m.max[3] - 10) < 1e-6 and math.abs(m.min[1] + 5) < 1e-6)
assert(math.abs(m.centre[3] - 5) < 1e-6)
```

`pc.doc.print_layout`: Where the print layout puts each copy: every part flat on its resting face, as many as the parts list prints, packed on the bed.

- `bed` (any, optional): {x, y, z}: the bed's width, depth and build height, mm; Preferences › Printing's when left out
- `gap` (number, optional): Between copies, mm; Preferences › Printing's (5) when left out
- Returns {plates, pieces = {{body, name, plate, min = {x, y, z}, max = {x, y, z}, transform}}, too_big, too_tall, unbuilt}

Notes:

- Nothing moves: the layout is where export (`file.export{layout = true}`) and the slicer put the copies. `transform` is the rigid row-major 4×4 matrix from the body's own frame to the bed; `min` and `max` are the copy's bounds there.
- A part rests on its largest flat face that has the whole part on one side; among faces of nearly that area, the one leaving it lowest. Parts too wide for the bed go on a plate of their own and are named in `too_big`; plates after the first lie beside it along X.

See also `pc.asm.parts`, `pc.asm.part`, `pc.file.export`.

Example: Three posts laid down on the bed.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 20}
local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 60}}.body
assert(#pc.doc.rebuild() == 0)
pc.asm.part{body = post, print = 3}
local layout = pc.doc.print_layout{bed = {x = 200, y = 200, z = 200}, gap = 5}
assert(layout.plates == 1 and #layout.pieces == 3)
for _, p in ipairs(layout.pieces) do
  assert(math.abs(p.min[3]) < 1e-3, "on the bed")
  assert(math.abs(p.max[3] - 10) < 1e-3, "lying on a 20 × 60 side")
  assert(p.min[1] >= 2.5 - 1e-3 and p.max[1] <= 197.5 + 1e-3)
end
```

`pc.doc.picture`: Write a PNG of the bodies, the same for the same arguments whatever the user's camera (except view "current").

- `path` (string): Where to write the PNG
- `view` (any, optional): Where to look from: "current" (the user's view direction; the default), "iso", "front", "back", "left", "right", "top", "bottom", {azimuth, elevation} in degrees (azimuth 0 the front, 90 the right side; elevation 90 from straight above), or {direction, up?}, the way the view looks. Always orthographic and framed to what is drawn
- `bodies` (list, optional): Only these bodies (ids or names), framed to them; every visible body when left out
- `highlight` (list, optional): Paint: a list of {body, faces?, edges?, color?}; faces by doc.faces index or name, edges by doc.edges index; the whole body when neither is given
- `markers` (list, optional): A dot and a label at world points: a list of {point, label?, color?}
- `section` (any, optional): Cut at the plane {origin, normal}: what lies on the side the normal points to is cut away, the cut drawn flat and darker
- `edges` (boolean, optional): Draw the faces' outlines (true)
- `xray` (boolean, optional): Bodies see-through, painted faces solid (false)
- `size` (list, optional): [width, height] in pixels (800 × 600, at most 2048 a side)
- `annotate` (boolean, optional): Draw an axis triad and the drawn bodies' box with its sizes (false)
- Returns {path, width, height}

Notes:

- It draws the solids as they stand: run `pc.doc.rebuild()` first. With no visible body it is refused ("Nothing is visible to draw"); hidden bodies are drawn only when named in `bodies`.
- Give a `view`: the default, "current", follows the user's view direction. A body named in `bodies` or `highlight` that does not exist, or a face past the body's last, is refused.
- Each side of `size` is kept between 16 and 2048 pixels. Folders missing from `path` are made.

See also `pc.doc.faces`, `pc.doc.edges`.

Example: A small isometric picture.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
assert(#pc.doc.rebuild() == 0)
local path = os.tmpname()
local shot = pc.doc.picture{path = path, view = "iso", size = {160, 120}}
assert(shot.path == path and shot.width == 160 and shot.height == 120)
local file = io.open(path, "rb")
assert(file:read(4) == "\137PNG")
file:close()
os.remove(path)
```

`pc.doc.parameters`: A feature's numbers that formulas set and read.

- `id` (id): The feature
- Returns a list of {name, key, label, kind, value, text, formula, error}: name is what formulas call it (nil when they cannot), value in mm or degrees

Notes:

- `name` or `key` is what `pc.doc.set_formula` and `pc.doc.set_value` take; `label` is only for showing. `value` is what the number comes to now, formulas worked out, and `text` shows it in the document's unit.
- A sketch lists only its named dimensions, so a sketch without any lists none. An id that is not a feature is refused ("no such feature").

See also `pc.doc.set_formula`, `pc.doc.set_value`.

Example: A pad's length as formulas see it.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local length
for _, p in ipairs(pc.doc.parameters{id = pad}) do
  if p.name == "length" then length = p end
end
assert(length.key == "/Pad/length" and length.kind == "length")
assert(length.value == 5 and length.text == "5 mm" and length.formula == nil)
```

`pc.doc.set_formula`: Set one of a feature's numbers by a formula, or take the formula away.

- `id` (id): The feature
- `parameter` (string): Its name or key, as doc.parameters lists them
- `formula` (string, optional): Such as "Printer.wall * 2"; nil takes it away
- Returns {value, text, error}: what it comes to

Notes:

- A formula that does not parse is refused. One that parses but does not work out (a missing name, a loop, an angle where a length is wanted) is kept: the answer has `error` and no `value`, the feature builds from the plain number its fields keep, and `pc.doc.rebuild()` does not list it. Check the answer's `error`.
- A bare number takes the parameter's unit: "12" is 12 mm for a length and 12 degrees for an angle. The fields keep their plain number; `pc.doc.feature`'s `values` show what the formula gives.
- Taking the formula away keeps the number as it stands: the fields take what the formula came to. A formula that did not work out leaves the plain number they had.
- It sets one feature's number; `pc.var.set` defines a variable that formulas read.

See also `pc.doc.parameters`, `pc.doc.set_value`, `pc.var.set`.

Example: A pad's length following a variable.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
pc.var.new{name = "Printer"}
pc.var.set{set = "Printer", name = "nozzle", formula = "0.4 mm"}
local got = pc.doc.set_formula{id = pad, parameter = "length", formula = "Printer.nozzle * 10"}
assert(math.abs(got.value - 4) < 1e-9 and got.error == nil)
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.volume - 800) < 1e-6)
pc.var.set{set = "Printer", name = "nozzle", formula = "0.6 mm"}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.volume - 1200) < 1e-6, "the pad follows")
local kept = pc.doc.set_formula{id = pad, parameter = "length"}
assert(math.abs(kept.value - 6) < 1e-9 and kept.formula == nil, "the 6 mm it came to stays")
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.volume - 1200) < 1e-6)
local bad = pc.doc.set_formula{id = pad, parameter = "length", formula = "30 deg"}
assert(bad.value == nil and bad.error:find("angle"), bad.error)
```

`pc.doc.set_value`: Set one of a feature's numbers, taking away any formula on it.

- `id` (id): The feature
- `parameter` (string): Its name or key, as doc.parameters lists them
- `value` (number): In millimetres or degrees

Notes:

- The value is not checked against what the feature accepts: a negative pad length is taken, and `pc.doc.rebuild()` reports it ("length must be positive").

See also `pc.doc.set_formula`, `pc.doc.parameters`.

Example: A length set by hand over a formula.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
pc.doc.set_formula{id = pad, parameter = "length", formula = "2 * 4 mm"}
pc.doc.set_value{id = pad, parameter = "length", value = 7}
local length = pc.doc.parameters{id = pad}[1]
assert(length.name == "length" and length.value == 7 and length.formula == nil)
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.volume - 1400) < 1e-6)
```

### var

`pc.var.new`: Make a variable set.

- `name` (string): What formulas call it: Printer.nozzle
- Returns the set's id

Notes:

- A name a feature or another set already has is refused ("something is already called ..."), and so is an empty one. A name with spaces is read in backticks: `` `My set`.width ``.
- The set is a feature with no body: `pc.doc.rename` renames it.

See also `pc.var.set`, `pc.var.list`.

Example: A set and one variable.

```lua
local set = pc.var.new{name = "Printer"}
pc.var.set{set = set, name = "nozzle", formula = "0.4 mm"}
local sets = pc.var.list()
assert(#sets == 1 and sets[1].id == set and sets[1].name == "Printer")
assert(pc.var.eval{formula = "Printer.nozzle"}.value == 0.4)
local ok, why = pcall(pc.var.new, {name = "Printer"})
assert(not ok and tostring(why):find("already called"), tostring(why))
```

`pc.var.set`: Set a variable to a formula, adding it when new.

- `set` (string): The set, by name or id
- `name` (string)
- `formula` (string): Such as "0.4 mm" or "3 * Printer.nozzle"
- `comment` (string, optional)
- Returns {value, text, error}: what it comes to

Notes:

- A formula that does not parse is refused; one that does not work out (a missing name, a loop, a length added to an angle) is kept, and the answer has `error` and no `value`.
- A bare number is a plain number, not a length: write the unit ("0.4 mm"). Setting a variable again keeps its comment unless a new one is given.
- Every formula reading it follows; `pc.doc.rebuild()` builds what moved. `pc.doc.set_formula` is what puts a formula on a feature's number.

See also `pc.var.eval`, `pc.doc.set_formula`, `pc.config.set`.

Example: A wall that follows the nozzle.

```lua
pc.var.new{name = "Printer"}
pc.var.set{set = "Printer", name = "nozzle", formula = "0.4 mm"}
local wall = pc.var.set{set = "Printer", name = "wall", formula = "3 * Printer.nozzle"}
assert(math.abs(wall.value - 1.2) < 1e-9 and wall.kind == "length")
pc.var.set{set = "Printer", name = "nozzle", formula = "0.6 mm", comment = "hardened steel"}
assert(math.abs(pc.var.eval{formula = "Printer.wall"}.value - 1.8) < 1e-9, "wall follows")
local loop = pc.var.set{set = "Printer", name = "loop", formula = "Printer.loop + 1"}
assert(loop.value == nil and loop.error:find("loop"), "kept, with its error")
```

`pc.var.remove`: Take a variable out of its set.

- `set` (string): The set, by name or id
- `name` (string)

Notes:

- It is not refused while formulas read it: they fail from then on ("Printer has no nozzle"). A configurations column naming it stays, and applies again to one added under that name.

See also `pc.var.rename`, `pc.config.remove_variable`.

Example: A variable taken away from under a formula.

```lua
pc.var.new{name = "Printer"}
pc.var.set{set = "Printer", name = "nozzle", formula = "0.4 mm"}
pc.var.set{set = "Printer", name = "wall", formula = "3 * Printer.nozzle"}
pc.var.remove{set = "Printer", name = "nozzle"}
local vars = pc.var.list{set = "Printer"}[1].variables
assert(#vars == 1 and vars[1].name == "wall")
assert(vars[1].value == nil and vars[1].error:find("no nozzle"), "what read it fails")
```

`pc.var.rename`: Rename a variable, and every formula that reads it.

- `set` (string): The set, by name or id
- `name` (string)
- `to` (string)

Notes:

- A name the set does not have, or a new name it already has, is refused. The configurations column follows the new name too.
- `pc.doc.rename` renames the set itself.

See also `pc.doc.rename`.

Example: A pad's formula follows the new name.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
pc.var.new{name = "Size"}
pc.var.set{set = "Size", name = "h", formula = "8 mm"}
pc.doc.set_formula{id = pad, parameter = "length", formula = "Size.h"}
pc.var.rename{set = "Size", name = "h", to = "height"}
assert(pc.doc.parameters{id = pad}[1].formula == "Size.height")
assert(pc.doc.parameters{id = pad}[1].value == 8)
```

`pc.var.list`: The variable sets and what each variable comes to.

- `set` (string, optional): Only this set, by name or id
- Returns a list of {id, name, variables}, each variable {name, formula, value, text, kind, error, comment}

Notes:

- `formula` is the variable's own; `value` is what it comes to now, which is the configuration's value while one in effect sets it. A variable whose formula does not work out has `error` and no `value`, `text` or `kind`.
- A `set` that does not exist is refused ("no variable set is called ...").

See also `pc.var.eval`, `pc.config.list`.

Example: Each variable's formula, value and comment.

```lua
pc.var.new{name = "Printer"}
pc.var.set{set = "Printer", name = "nozzle", formula = "0.4 mm", comment = "brass"}
pc.var.set{set = "Printer", name = "angle", formula = "45"}
local vars = pc.var.list{set = "Printer"}[1].variables
assert(vars[1].name == "nozzle" and vars[1].formula == "0.4 mm" and vars[1].comment == "brass")
assert(vars[1].value == 0.4 and vars[1].kind == "length" and vars[1].text == "0.4 mm")
assert(vars[2].kind == "number" and vars[2].value == 45, "a bare number has no unit")
```

`pc.var.eval`: What a formula comes to in this document.

- `formula` (string)
- Returns {value, kind, text}: value in mm or degrees

Notes:

- It changes nothing. A formula that does not work out is refused with the reason ("nothing is called Nope"), where `pc.var.set` would keep it.
- `kind` is such as length, angle, number or area; a bare number is a plain number, and trigonometry takes degrees. It reads the configuration in effect.

See also `pc.var.set`, `pc.var.list`.

Example: Units worked out.

```lua
pc.var.new{name = "Printer"}
pc.var.set{set = "Printer", name = "nozzle", formula = "0.4 mm"}
local wall = pc.var.eval{formula = "3 * Printer.nozzle + 1 in"}
assert(math.abs(wall.value - 26.6) < 1e-9 and wall.kind == "length")
local area = pc.var.eval{formula = "2 mm * 3 mm"}
assert(area.kind == "area" and area.value == 6)
assert(math.abs(pc.var.eval{formula = "sin(30)"}.value - 0.5) < 1e-12, "degrees")
```

### config

`pc.config.list`: The configurations: the variables they set, each row, and which is in effect.

- Returns {columns, rows: [{name, values, left_out}], active}

Notes:

- Each row's `values` line up with `columns`; an empty string leaves that variable its own formula. `left_out` lists the ids of the bodies the row leaves out, and `active` is nil while no configuration is in effect.

See also `pc.config.activate`, `pc.var.list`.

Example: Two sizes, the large one in effect.

```lua
pc.var.new{name = "Size"}
pc.var.set{set = "Size", name = "width", formula = "40 mm"}
pc.config.add_variable{variable = "Size.width"}
pc.config.new{name = "Small"}
pc.config.set{name = "Small", variable = "Size.width", value = "30 mm"}
pc.config.new{name = "Large", like = "Small"}
pc.config.set{name = "Large", variable = "Size.width", value = "60 mm"}
pc.config.activate{name = "Large"}
local t = pc.config.list()
assert(t.columns[1] == "Size.width" and t.active == "Large")
assert(t.rows[1].name == "Small" and t.rows[1].values[1] == "30 mm")
assert(t.rows[2].name == "Large" and t.rows[2].values[1] == "60 mm")
assert(pc.var.eval{formula = "Size.width"}.value == 60)
```

`pc.config.new`: Add a configuration.

- `name` (string): Such as "Large"
- `like` (string, optional): Start from this configuration's values

Notes:

- The first one makes the document's configurations table. A new configuration is not put in effect (`pc.config.activate` does that), and without `like` it leaves every variable its own.
- A name already used, or a `like` that does not exist, is refused.

See also `pc.config.set`, `pc.config.activate`.

Example: A configuration copied from another.

```lua
pc.var.new{name = "Size"}
pc.var.set{set = "Size", name = "width", formula = "40 mm"}
pc.config.add_variable{variable = "Size.width"}
pc.config.new{name = "Large"}
pc.config.set{name = "Large", variable = "Size.width", value = "60 mm"}
pc.config.new{name = "Large, thin", like = "Large"}
local rows = pc.config.list().rows
assert(#rows == 2 and rows[2].values[1] == "60 mm", "copied from Large")
assert(pc.config.list().active == nil, "a new configuration is not put in effect")
```

`pc.config.remove`: Remove a configuration.

- `name` (string)

Notes:

- Removing the one in effect leaves none in effect: every variable goes back to its own formula. A name that does not exist is refused.

See also `pc.config.activate`.

Example: The configuration in effect removed.

```lua
pc.var.new{name = "Size"}
pc.var.set{set = "Size", name = "width", formula = "40 mm"}
pc.config.add_variable{variable = "Size.width"}
pc.config.new{name = "Large"}
pc.config.set{name = "Large", variable = "Size.width", value = "60 mm"}
pc.config.activate{name = "Large"}
pc.config.remove{name = "Large"}
assert(#pc.config.list().rows == 0 and pc.config.list().active == nil)
assert(pc.var.eval{formula = "Size.width"}.value == 40, "the variable's own formula again")
```

`pc.config.rename`: Rename a configuration.

- `name` (string)
- `to` (string)

Notes:

- The configuration in effect stays in effect under its new name. A name that does not exist, or a new name already used, is refused.

Example: The one in effect renamed.

```lua
pc.config.new{name = "Big"}
pc.config.activate{name = "Big"}
pc.config.rename{name = "Big", to = "Large"}
local t = pc.config.list()
assert(t.rows[1].name == "Large" and t.active == "Large")
```

`pc.config.add_variable`: Let the configurations set a variable: it becomes a column.

- `variable` (string): As formulas read it: Size.width

Notes:

- The variable must exist, written as `Set.name` ("write it as Set.name"); a column already there is refused. Every configuration starts with an empty value in it, leaving the variable its own formula.

See also `pc.config.set`, `pc.var.set`.

Example: A column, empty in every row.

```lua
pc.var.new{name = "Size"}
pc.var.set{set = "Size", name = "width", formula = "40 mm"}
pc.config.new{name = "Large"}
pc.config.add_variable{variable = "Size.width"}
local t = pc.config.list()
assert(t.columns[1] == "Size.width" and t.rows[1].values[1] == "", "empty: the variable's own")
local ok, why = pcall(pc.config.add_variable, {variable = "width"})
assert(not ok and tostring(why):find("Set.name"), tostring(why))
```

`pc.config.remove_variable`: Take a variable's column away.

- `variable` (string): As formulas read it: Size.width

Notes:

- The variable stays in its set and goes back to its own formula. A variable that is not a column is refused.

See also `pc.config.add_variable`, `pc.var.remove`.

Example: The column gone, the variable its own again.

```lua
pc.var.new{name = "Size"}
pc.var.set{set = "Size", name = "width", formula = "40 mm"}
pc.config.add_variable{variable = "Size.width"}
pc.config.new{name = "Large"}
pc.config.set{name = "Large", variable = "Size.width", value = "60 mm"}
pc.config.activate{name = "Large"}
assert(pc.var.eval{formula = "Size.width"}.value == 60)
pc.config.remove_variable{variable = "Size.width"}
assert(#pc.config.list().columns == 0)
assert(pc.var.eval{formula = "Size.width"}.value == 40, "its own formula again")
```

`pc.config.set`: What a configuration gives a variable: a formula, or empty for its own.

- `name` (string): The configuration
- `variable` (string): As formulas read it: Size.width
- `value` (string): Such as "60 mm"

Notes:

- The variable must be a column (`pc.config.add_variable`). The value is checked only for its syntax: one that reads the same variable is a loop, which shows as the variable's `error` once the configuration is in effect.
- It changes the variable's value only while the configuration is in effect; `pc.var.set` changes the variable's own formula.

See also `pc.config.add_variable`, `pc.config.activate`, `pc.var.set`.

Example: A value given, then left to the variable.

```lua
pc.var.new{name = "Size"}
pc.var.set{set = "Size", name = "width", formula = "40 mm"}
pc.config.add_variable{variable = "Size.width"}
pc.config.new{name = "Large"}
pc.config.set{name = "Large", variable = "Size.width", value = "1.5 * 40 mm"}
pc.config.activate{name = "Large"}
assert(pc.var.eval{formula = "Size.width"}.value == 60)
pc.config.set{name = "Large", variable = "Size.width", value = ""}
assert(pc.var.eval{formula = "Size.width"}.value == 40, "empty: the variable's own")
```

`pc.config.leave_out`: The bodies a configuration leaves out: not drawn, picked, exported or checked.

- `name` (string): The configuration
- `bodies` (list): The bodies' ids; an empty list leaves none out

Notes:

- The list replaces the one the configuration had. An id that is not a body of the document is refused, and so is a configuration that does not exist.
- The bodies are left out only while the configuration is in effect; they stay in the document.

See also `pc.config.list`, `pc.config.activate`.

Example: A lid left out of one configuration.

```lua
local function box(z)
  local s = pc.sketch.new{plane = "XY", offset = z}
  pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
end
local base, lid = box(0), box(5)
assert(#pc.doc.rebuild() == 0)
pc.config.new{name = "Open"}
pc.config.leave_out{name = "Open", bodies = {lid}}
assert(pc.config.list().rows[1].left_out[1] == lid)
pc.config.leave_out{name = "Open", bodies = {}}
assert(#pc.config.list().rows[1].left_out == 0, "an empty list leaves none out")
local ok, why = pcall(pc.config.leave_out, {name = "Closed", bodies = {lid}})
assert(not ok, "no configuration is called Closed")
```

`pc.config.activate`: Put a configuration in effect.

- `name` (string, optional): Nil leaves every variable its own

Notes:

- Every variable the configuration gives a value takes it, and what reads them follows once `pc.doc.rebuild()` builds it. A name that does not exist is refused.

See also `pc.config.list`, `pc.config.set`.

Example: A pad's length switched by configuration.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
pc.var.new{name = "Size"}
pc.var.set{set = "Size", name = "height", formula = "5 mm"}
pc.doc.set_formula{id = pad, parameter = "length", formula = "Size.height"}
pc.config.add_variable{variable = "Size.height"}
pc.config.new{name = "Tall"}
pc.config.set{name = "Tall", variable = "Size.height", value = "12 mm"}
pc.config.activate{name = "Tall"}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.volume - 2400) < 1e-6)
pc.config.activate{}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.volume - 1000) < 1e-6, "its own 5 mm again")
```

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

- `path` (string, optional): The STEP, IGES, STL, OBJ, 3MF, PLY, glTF or VRML file; the dialog when left out
- Returns nothing; pc.doc.rebuild() waits for the import

`pc.file.export`: Export.

- `path` (string, optional): Where to write; the dialog when left out
- `format` (string, optional): step, step_nurbs (every surface a spline), stl or 3mf; from the path's extension when left out
- `bodies` (list, optional): The bodies to write; every visible one when left out
- `tolerance` (number, optional): The mesh formats' distance to the true surface, mm (0.01)
- `layout` (boolean, optional): The print layout (doc.print_layout) rather than the bodies where they sit
- Returns {path, written, skipped, triangles}

`pc.file.send_to_slicer`: Send to slicer.

- `layout` (boolean, optional): The print layout rather than the bodies where they sit; Preferences › Printing says when left out

`pc.file.print_layout`: Print layout.

### edit

`pc.edit.undo`: Undo.

`pc.edit.redo`: Redo.

`pc.edit.cut`: Cut.

`pc.edit.copy`: Copy.

`pc.edit.paste`: Paste.

`pc.edit.rename`: Rename the selected item.

`pc.edit.properties`: Properties of the selected item.

`pc.edit.recompute`: Recompute all.

`pc.edit.repeat`: Repeat the last tool.

### view

`pc.view.fit_all`: Fit all.

`pc.view.fit_selection`: Fit selection.

`pc.view.isolate`: Show only the selected body.

`pc.view.show_all`: Show every body.

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

`pc.view.annotations`: Annotations.

`pc.view.grid`: Grid.

`pc.view.origin_planes`: Origin planes.

### tab

`pc.tab.new`: New tab.

`pc.tab.close`: Close tab.

`pc.tab.reopen`: Reopen closed tab.

`pc.tab.next`: Next tab.

`pc.tab.previous`: Previous tab.

### sketch

`pc.sketch.new`: Make an empty sketch on a base plane.

- `body` (id, optional): The body it belongs to; the selected body, else a new one
- `plane` (string, optional): XY (the default), XZ or YZ
- `offset` (number, optional): How far along the plane's normal it sits
- `name` (string, optional): Its name in the tree
- `attachment` (any, optional): Attached as a datum plane is, the attachment as a datum keeps it (doc.feature on a datum shows it): {Face = {face = {point = {x, y, z}, normal = {x, y, z}}}}, {ThreePoints = {points = {{At = {point = {x, y, z}}}, ...}}} and the like, faces and edges on the body's own solid; the sketch follows what it stands on. design.datum makes the same from plainer arguments, and `on` takes that datum
- `attachment_offset` (any, optional): The attachment's offset, as a datum's
- `on` (id, optional): A datum plane, or a coordinate system whose XY, XZ or YZ plane (see plane) it takes
- `normal` (list, optional): A plane of its own instead: its normal as {x, y, z}
- `origin` (list, optional): With normal: where the plane's origin sits, {x, y, z}
- `x_axis` (list, optional): With normal: the sketch's X direction, {x, y, z}
- `generator` (string, optional): gear, sprocket, shaft or keyway: the sketch is that generator's, at its default numbers, centred on the plane's origin
- Returns the sketch's id

Notes:

- Without `body` the sketch goes in the selected body, else in a new one: in a script two sketches made without `body` land in two bodies, and a pocket or a hole from the second is refused for want of material. Give `body` from `pc.doc.feature{id = ...}.body` or `pc.doc.new_body`.
- `body` takes any body, a surface body too: that is how a sketch starts in a surface body, as the Surface bench's Create sketch does.
- XY faces +Z, YZ faces +X and XZ faces -Y, so a pad from an XZ sketch grows toward -Y and `offset` moves an XZ sketch toward -Y. The sketch's x and y run along the plane's two letters (on XZ, y is world Z). Lower case names are taken too.
- With `generator` it is named after it (Sprocket, Sprocket_1, ...) and `sketch.generator` sets its numbers, as `design.sprocket` and its kin do on a base plane or a picked face.

See also `pc.doc.new_body`, `pc.sketch.rect`, `pc.design.datum`.

Example: Two sketches in one body.

```lua
local body = pc.doc.new_body{name = "Bracket"}
local base = pc.sketch.new{body = body, plane = "XY"}
local side = pc.sketch.new{body = body, plane = "XZ", offset = 5}
assert(pc.doc.feature{id = base}.body == body)
assert(pc.doc.feature{id = side}.body == body)
assert(#pc.doc.bodies() == 1, "both sketches went in the one body")
```

`pc.sketch.import_dxf`: Make a sketch of a DXF drawing: its lines, arcs, circles, ellipses and polylines as sketch curves, splines as lines through points on them, hidden ones as construction, ends that meet sharing one point.

- `body` (id, optional): The body it belongs to; the selected body, else a new one
- `plane` (string, optional): XY (the default), XZ or YZ
- `offset` (number, optional): How far along the plane's normal it sits
- `name` (string, optional): Its name in the tree
- `attachment` (any, optional): Attached as a datum plane is, the attachment as a datum keeps it (doc.feature on a datum shows it): {Face = {face = {point = {x, y, z}, normal = {x, y, z}}}}, {ThreePoints = {points = {{At = {point = {x, y, z}}}, ...}}} and the like, faces and edges on the body's own solid; the sketch follows what it stands on. design.datum makes the same from plainer arguments, and `on` takes that datum
- `attachment_offset` (any, optional): The attachment's offset, as a datum's
- `on` (id, optional): A datum plane, or a coordinate system whose XY, XZ or YZ plane (see plane) it takes
- `normal` (list, optional): A plane of its own instead: its normal as {x, y, z}
- `origin` (list, optional): With normal: where the plane's origin sits, {x, y, z}
- `x_axis` (list, optional): With normal: the sketch's X direction, {x, y, z}
- `path` (string): The DXF file
- `scale` (number, optional): Millimetres per drawing unit; the drawing's own unit when left out, else 1
- Returns the sketch's id

Notes:

- Left out, `scale` follows the drawing's own unit: a drawing in inches comes in at 25.4 mm a unit. A drawing that names no unit comes in at 1 mm a unit.
- It is placed as `sketch.new` places a sketch: without `body` it goes in the selected body, else a new one. It is named after the file unless `name` says otherwise.
- A file that cannot be read or parsed is refused, and so is a drawing with no curves.

See also `pc.sketch.new`, `pc.sketch.repair`.

`pc.sketch.image`: Lay a picture (PNG or JPEG) on a sketch's plane to draw over: in the sketch given or being edited, else in a new one.

- `body` (id, optional): The body it belongs to; the selected body, else a new one
- `plane` (string, optional): XY (the default), XZ or YZ
- `offset` (number, optional): How far along the plane's normal it sits
- `name` (string, optional): Its name in the tree
- `attachment` (any, optional): Attached as a datum plane is, the attachment as a datum keeps it (doc.feature on a datum shows it): {Face = {face = {point = {x, y, z}, normal = {x, y, z}}}}, {ThreePoints = {points = {{At = {point = {x, y, z}}}, ...}}} and the like, faces and edges on the body's own solid; the sketch follows what it stands on. design.datum makes the same from plainer arguments, and `on` takes that datum
- `attachment_offset` (any, optional): The attachment's offset, as a datum's
- `on` (id, optional): A datum plane, or a coordinate system whose XY, XZ or YZ plane (see plane) it takes
- `normal` (list, optional): A plane of its own instead: its normal as {x, y, z}
- `origin` (list, optional): With normal: where the plane's origin sits, {x, y, z}
- `x_axis` (list, optional): With normal: the sketch's X direction, {x, y, z}
- `x` (number, optional): Its middle, mm; 0 when left out
- `y` (number, optional): Its middle, mm; 0 when left out
- `width` (number, optional): How wide it lies, mm
- `angle` (number, optional): Its turn counter-clockwise, degrees
- `opacity` (number, optional): How much of it shows, 0 to 1
- `path` (string): The picture file
- `sketch` (id, optional): The sketch it goes in
- Returns {sketch, image}

Notes:

- Without `sketch` it goes in the sketch being edited, else in a new sketch named after the file and placed as `sketch.new` places one (`body`, `plane`, `on`).
- It lies 100 mm wide, centred on (0, 0), at opacity 0.5, unless `width`, `x`, `y` or `opacity` say otherwise; `angle` is degrees counter-clockwise.
- The file is kept in the document. The picture only draws: no profile and no constraint come of it.
- A file that cannot be read, or is not a PNG or JPEG, is refused. The `image` returned is what `sketch.set_image` takes.

See also `pc.sketch.set_image`.

`pc.sketch.set_image`: Move, size, turn or fade a sketch's picture, or take it away.

- `sketch` (id): The sketch to draw in
- `x` (number, optional): Its middle, mm; 0 when left out
- `y` (number, optional): Its middle, mm; 0 when left out
- `width` (number, optional): How wide it lies, mm
- `angle` (number, optional): Its turn counter-clockwise, degrees
- `opacity` (number, optional): How much of it shows, 0 to 1
- `image` (id): The picture
- `remove` (boolean, optional): true: take it away

Notes:

- `image` is the id `sketch.image` returned, and `sketch` must be the sketch holding it, or it is refused.
- Only what is given changes. `width` must be more than 0, `opacity` is held between 0 and 1, and `angle` is degrees counter-clockwise.

See also `pc.sketch.image`.

`pc.sketch.point`: Add a point.

- `sketch` (id): The sketch to draw in
- `x` (number)
- `y` (number)
- Returns the point's id

Notes:

- A point the sketch already has exactly at (x, y) is returned rather than a second one made.
- Lines, arcs and circles drawn later with an end or a centre exactly on it take it: making the points first is how a script knows the ids of the ends it constrains.

See also `pc.sketch.constrain`, `pc.sketch.geometry`.

Example: A corner made first and shared.

```lua
local s = pc.sketch.new{plane = "XY"}
local corner = pc.sketch.point{sketch = s, x = 10, y = 5}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 5}
local ends = 0
for _, g in ipairs(pc.doc.feature{id = s}.fields.sketch.geometry) do
  if g.Line and (g.Line.start == corner or g.Line["end"] == corner) then ends = ends + 1 end
end
assert(ends == 2, "the rectangle's corner is the point made first")
```

`pc.sketch.line`: Add a line from (x1, y1) to (x2, y2).

- `sketch` (id): The sketch to draw in
- `x1` (number)
- `y1` (number)
- `x2` (number)
- `y2` (number)
- Returns the line's id

Notes:

- Each end takes a point the sketch has exactly there, else a new one: lines drawn end to end share their ends and close a profile.
- Unlike `sketch.polyline` and `sketch.rect`, it adds no constraint: a level line is not held level.
- Its ends are points of their own: the line's `start` and `end` in `pc.doc.feature{id = s}.fields.sketch.geometry`, or points made first with `sketch.point`. Two ends at the same spot are refused ("a line needs two different ends").

See also `pc.sketch.polyline`, `pc.sketch.point`.

Example: Three lines end to end close a triangle.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 30, y2 = 0}
pc.sketch.line{sketch = s, x1 = 30, y1 = 0, x2 = 0, y2 = 20}
pc.sketch.line{sketch = s, x1 = 0, y1 = 20, x2 = 0, y2 = 0}
assert(#pc.sketch.constraints{sketch = s} == 0, "nothing holds a line level")
local pad = pc.design.pad{sketch = s, length = 4}
assert(#pc.doc.rebuild() == 0, "the ends are shared, so the triangle closes")
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.volume - 30 * 20 / 2 * 4) < 1e-3)
```

`pc.sketch.polyline`: Add lines through a list of points, each ending where the next starts; a level or upright one is held so.

- `sketch` (id): The sketch to draw in
- `points` (list): Points as {x, y} pairs
- `closed` (boolean, optional): Join the last point to the first
- Returns the lines' ids

Notes:

- A point is `{x, y}` or `{x = .., y = ..}`. Ending on the first point closes the outline as `closed = true` does: an end landing exactly on a point the sketch has takes that point.
- Only closed loops count in a profile: an open polyline is left out of what a pad or pocket uses, and a sketch with no closed loop fails at `pc.doc.rebuild()` with "profile is not closed", not when the feature is made.

See also `pc.sketch.rect`, `pc.sketch.line`.

Example: A closed triangle padded.

```lua
local s = pc.sketch.new{plane = "XY"}
local lines = pc.sketch.polyline{sketch = s, points = {{0, 0}, {30, 0}, {0, 20}}, closed = true}
assert(#lines == 3)
local pad = pc.design.pad{sketch = s, length = 5}
assert(#pc.doc.rebuild() == 0, "the triangle closes")
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.volume - 30 * 20 / 2 * 5) < 1e-3)
```

`pc.sketch.rect`: Add a rectangle from its corner (x, y), its width and its height, its sides held level and upright.

- `sketch` (id): The sketch to draw in
- `x` (number)
- `y` (number)
- `width` (number)
- `height` (number)
- Returns the four lines' ids

Notes:

- (x, y) is a corner, not the centre: a rectangle centred on the origin starts at (-width / 2, -height / 2). A negative width or height draws it to the left or below.
- Its sides are held level and upright but carry no dimensions; `pc.sketch.constrain` adds them.

See also `pc.sketch.polyline`, `pc.sketch.constrain`.

Example: A plate centred on the origin.

```lua
local s = pc.sketch.new{plane = "XY"}
local sides = pc.sketch.rect{sketch = s, x = -20, y = -15, width = 40, height = 30}
assert(#sides == 4)
local pad = pc.design.pad{sketch = s, length = 3}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.volume - 40 * 30 * 3) < 1e-3)
assert(math.abs(m.centre[1]) < 1e-6 and math.abs(m.centre[2]) < 1e-6, "centred")
```

`pc.sketch.circle`: Add a circle.

- `sketch` (id): The sketch to draw in
- `x` (number): The centre
- `y` (number): The centre
- `radius` (number)
- Returns the circle's id

Notes:

- It takes the radius, not the diameter; a radius of 0 or less is refused.
- A circle inside a closed outline of the same sketch is a hole in what is padded from it; circles apart from each other pad as separate solids in one body.
- `pc.design.hole` reads only a circle's centre: the hole's size is its own `diameter`, whatever the circle's radius.

See also `pc.design.hole`.

Example: A washer: a ring padded from two circles.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 10}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 4}
local pad = pc.design.pad{sketch = s, length = 2}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.volume - math.pi * (100 - 16) * 2) < 0.01, m.volume)
```

`pc.sketch.arc`: Add an arc, counter-clockwise from the start angle to the end angle.

- `sketch` (id): The sketch to draw in
- `x` (number): The centre
- `y` (number): The centre
- `radius` (number)
- `start` (number): Degrees from the sketch's X axis
- `end` (number): Degrees from the sketch's X axis
- Returns the arc's id

Notes:

- Angles are degrees, and the arc runs counter-clockwise from `start` to `end`: -90 to 90 is the right half, 90 to -90 the left.
- It takes the radius, not the diameter; a radius of 0 or less is refused.
- Its centre and both ends are points of their own, each taking a point the sketch has exactly there, so a line drawn to an end joins it.

See also `pc.sketch.circle`, `pc.sketch.draw`.

Example: A half disc closed by a line.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.arc{sketch = s, x = 0, y = 0, radius = 10, start = -90, ["end"] = 90}
pc.sketch.line{sketch = s, x1 = 0, y1 = 10, x2 = 0, y2 = -10}
local pad = pc.design.pad{sketch = s, length = 2}
assert(#pc.doc.rebuild() == 0, "the line ends on the arc's ends")
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.volume - math.pi * 100 / 2 * 2) < 0.01, m.volume)
assert(m.min[1] > -1e-6, "counter-clockwise from -90 to 90 is the right half")
```

`pc.sketch.geometry`: List the sketch's elements with their points.

- `sketch` (id): The sketch to draw in
- Returns a list of {id, kind, points, radius?, construction}

Notes:

- A line's points are its start and end, an arc's its centre, start and end, a circle's and an ellipse's their centre; positions are as last solved, with the values formulas give its dimensions (what builds, which `pc.doc.feature{id = s}.fields` may not yet be). Each element also says whether it is `external`.
- Ends and centres are listed again as elements of kind point. A spline, a parabola or a hyperbola is kind "other" with no points: its control points are in `pc.doc.feature{id = s}.fields.sketch.geometry`.

See also `pc.sketch.constraints`, `pc.doc.feature`.

Example: Ends and centres are points of their own.

```lua
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
```

`pc.sketch.constrain`: Constrain elements, as the constraint's toolbar button does for a selection.

- `sketch` (id): The sketch to draw in
- `kind` (string): coincident, point_on_object, midpoint, horizontal, vertical, horizontal_vertical, parallel, perpendicular, tangent, equal, symmetric, block, group, lock, dimension, distance, distance_x, distance_y, gap, arc_length, radius, diameter, radius_diameter, angle, angle_x, angle_y, angle_at_point, arc_angle, angle_three_points (items: arm, corner, arm), ellipse_minor or refraction; radius on an ellipse is its major radius, arc_length on a spline or conic its length
- `items` (list): Element ids, or "origin", "x_axis" and "y_axis"
- `value` (number, optional): A dimension's value (mm, degrees for an angle, the ratio of indices for a refraction); the measured one when left out
- `remove_redundant` (boolean, optional): Take away the older constraints the new ones make redundant
- Returns the new constraints' ids

Notes:

- Points are elements of their own: a line's ends are the point ids at its `start` and `end` in `pc.doc.feature{id = s}.fields.sketch.geometry`, the line's own id is the line. A point made with `sketch.point` before the line is the same id as the end drawn on it.
- Without `value` a dimension takes what it measures on the sketch as it stands. Angles are degrees, a diameter the diameter, a radius the radius.
- "distance" on one line is its length (listed as Length), on two points the distance between them, on a point and a curve or two curves the gap. "dimension" picks as the toolbar does: a line's length, a circle's diameter, an arc's radius, two lines' angle (their distance when parallel).
- Nothing stays put until constrained: a dimension on free geometry moves every item it names, so tie a corner to "origin" first to keep it where it was drawn.
- "lock" holds a point by its distances along X and Y from the origin, each the size of a coordinate (a point at x = -5 takes 5 and stays at -5).
- "group" makes its items one rigid unit, keeping the shape they have now: it moves and turns as a whole, three degrees of freedom, and a drag of any member carries all of it. Constraints among the members alone that say nothing of the axes rest while the group holds; of those that do (horizontal, vertical, an angle to an axis), the first turns the whole group and the rest rest.
- A constraint that contradicts others is still added; `pc.sketch.status` names the conflict. While it stands the sketch does not solve, gives no profile, and what is built from it fails at `pc.doc.rebuild()` ("the sketch does not solve: its constraints conflict"). `remove_redundant` takes away only older constraints the new one repeats, never one it contradicts.
- A kind that does not fit the items is refused ("the ... constraint does not fit these items"), one that is no kind at all is refused with the list of kinds, and so is a `value` for a kind that takes none.

See also `pc.sketch.status`, `pc.sketch.set_value`, `pc.sketch.constraints`.

Example: A plate fully constrained from its corner on the origin.

```lua
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
```

`pc.sketch.set_value`: Change a dimension's value.

- `sketch` (id): The sketch to draw in
- `constraint` (id)
- `value` (number): mm, or degrees for an angle
- `driving` (boolean, optional): false makes it a reference dimension that only measures

Notes:

- `constraint` is an id `sketch.constrain` returned; an element's id, or a constraint that is not a dimension, is refused.
- Angles are degrees; a diameter takes the diameter, a radius the radius.
- To bind a dimension to a formula, `doc.set_formula` takes it by the key `doc.parameters` lists for the sketch, which is the constraint's id.

See also `pc.sketch.constrain`, `pc.doc.parameters`, `pc.doc.set_formula`.

Example: A circle's diameter changed after it was dimensioned.

```lua
local s = pc.sketch.new{plane = "XY"}
local c = pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 5}
local d = pc.sketch.constrain{sketch = s, kind = "diameter", items = {c}}
pc.sketch.set_value{sketch = s, constraint = d[1], value = 20}
assert(pc.sketch.constraints{sketch = s}[1].value == 20)
local pad = pc.design.pad{sketch = s, length = 1}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.volume - math.pi * 10 ^ 2) < 0.01, m.volume)
```

`pc.sketch.draw`: Run a drawing or editing tool over points of the sketch, as clicks there would.

- `sketch` (id): The sketch to draw in
- `tool` (string): line, polyline, rect, rect_center, rect_rounded, rect3, rect_center3, rect_frame, circle, circle3, arc, arc3, ellipse, ellipse3, ellipse_arc, parabola, hyperbola, bspline, polygon, slot, arc_slot, point, fillet, chamfer, trim, extend, split, bspline_knot, offset, translate, rotate, scale or mirror
- `points` (list): The clicks, each {x, y}, or {x = , y = , typed = {length = 20}, constrain = true} with values typed at it, or for the line tool {x = , y = , arc = true}: an arc there, tangent to what ends where it draws from; "arc" and "line" switch a polyline, "finish" ends a spline
- `tolerance` (number, optional): How close a click snaps onto points and curves, mm (0.001)
- `params` (any, optional): Tool settings: polygon_sides, slot_width, fillet_radius, chamfer_length, corner_keep, offset_distance, offset_round, offset_both, offset_delete, offset_linked, copies, copies_linked, copies_total, copies_symmetric, rotate_full_turn, bspline_periodic, bspline_degree, bspline_interpolate, auto_constraints, mirror_keep, mirror_linked, mirror_center, cut_at_axes
- `construction` (boolean, optional): What it makes is construction geometry
- `avoid_redundant` (boolean, optional): Drop auto constraints that add nothing (true)
- `selection` (list, optional): The elements offset, translate, rotate, scale and mirror act on
- Returns {elements, constraints}: what it made

Notes:

- Each click is in the sketch's millimetres and snaps as a click in the view does: one on the origin is held there and one on an axis is held on it, the constraints coming with what is made.
- A tool takes the clicks its shape needs: line, circle (centre, then a point on it) and slot (the ends of its centre line) two; rect_center its centre, then a corner; ellipse its centre, an end of the major axis, then a point on it. Clicks short of a shape make nothing, without an error.
- Sizes not clicked come from `params`: `slot_width` 4 mm, `polygon_sides` 6, `fillet_radius` and `chamfer_length` 2 mm when not given. A fillet or chamfer takes one click on the corner. A bspline ends with the word "finish" in `points`.
- A value typed at a click (`typed = {length = 20}`) sets the shape's size; with `constrain = true` it is kept as a dimension. The names are length, angle, width, height, diameter, radius, major_radius, minor_radius, sweep (an arc's turn, degrees counter-clockwise) and x and y (a shape's first point).
- It returns every element made, end points and centres included, and every constraint made with them.

See also `pc.sketch.polyline`, `pc.sketch.rect`, `pc.sketch.circle`.

Example: A slot drawn by its centre line.

```lua
local s = pc.sketch.new{plane = "XY"}
local made = pc.sketch.draw{sketch = s, tool = "slot", points = {{0, 0}, {20, 0}}, params = {slot_width = 6}}
assert(#made.elements > 0 and #made.constraints > 0)
local pad = pc.design.pad{sketch = s, length = 1}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.volume - (20 * 6 + math.pi * 3 ^ 2)) < 1e-3, m.volume)
```

`pc.sketch.drag`: Drag elements by a step, the rest of the sketch following its constraints.

- `sketch` (id): The sketch to draw in
- `items` (list): The elements to drag
- `by` (list): The step, {x, y}

Notes:

- `by` is a step {dx, dy} in mm, not a place to go to: the items move by it as far as their constraints let them, and the rest of the sketch follows.
- A point held by "lock" stays where it is. A corner of a rectangle from `sketch.rect` stretches it, the opposite corner staying put.
- Dragging a text block's point moves the whole text.

See also `pc.sketch.set_value`.

Example: A rectangle stretched by its corner.

```lua
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
```

`pc.sketch.attachment`: Move a sketch on the datum it is attached to: along its normal, across it, turned about it.

- `offset` (number, optional): Along the normal, mm
- `shift` (list, optional): Across the plane, {x, y} in mm
- `turn` (number, optional): About the normal, degrees
- `sketch` (id): The sketch to draw in

Notes:

- Only a sketch made with `on` (a datum plane or coordinate system) takes it; one on a base plane, a plane of its own or an `attachment` is refused ("is not attached to a datum").
- Each value given replaces the one the sketch had rather than adding to it: `offset = 5` twice leaves it 5 mm off the datum.
- `shift` runs along the datum's own x and y; `turn` is degrees about its normal.

See also `pc.sketch.new`, `pc.design.datum`, `pc.sketch.set_plane`.

Example: A sketch set 5 mm off its datum.

```lua
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
```

`pc.sketch.external_from`: Bring another sketch's curves and points, or a datum, into this sketch as external geometry that follows them.

- `from` (id): A sketch or a datum
- `counts` (boolean, optional): true: it counts in the profile, as drawn geometry does; false (the default): it only guides the sketch
- `sketch` (id): The sketch to draw in
- Returns the external elements made

Notes:

- Every curve and loose point of the other sketch comes, its construction left out; a datum comes as one element.
- It only guides unless `counts = true`: a sketch holding guides alone has no profile, and a pad of it fails at `pc.doc.rebuild()`.
- It returns {elements, constraints}: the curves and the points at their ends. Only the curves are external geometry, which `pc.sketch.geometry` marks `external`.
- `from` must be a sketch other than this one, or a datum; anything else is refused.
- The elements follow their source without this sketch being opened: when the other sketch or the datum moves (by hand or by a formula), `pc.sketch.geometry` reads them where it now is and the next `pc.doc.rebuild()` builds from that. A source curve that becomes another kind of curve, or is deleted, is caught up with when this sketch is next edited.
- What the sketch already holds comes once: bringing the same sketch again adds only what is new in it, and is refused ("... already in the sketch ...") when nothing is.

See also `pc.sketch.external_defining`, `pc.sketch.carbon_copy`.

Example: A circle from the sketch below, counted, padded and following its source.

```lua
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
```

`pc.sketch.external_defining`: Count external geometry in the sketch's profiles, or leave it only guiding.

- `items` (list): External elements' ids
- `on` (boolean, optional): true counts them (the default), false stops
- `sketch` (id): The sketch to draw in

Notes:

- Every item must be external geometry, or the call is refused ("... is not external geometry"); the elements `pc.sketch.geometry` marks `external` are. The points at an external curve's ends and centre go with their curve, so the `elements` `sketch.external`, `sketch.external_from` and `sketch.intersection` return can be passed as they come.
- Counting takes an element out of construction; `on = false` makes it a guide again.

See also `pc.sketch.external_from`, `pc.sketch.external`.

Example: A guide made to count.

```lua
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
```

`pc.sketch.solver_settings`: How far the solver goes on this sketch.

- `iterations` (number, optional): The most steps it takes (100 when never set)
- `tolerance` (number, optional): How small what is left must be, against the sketch's size (1e-9 when never set)
- `sketch` (id): The sketch to draw in

Notes:

- `iterations` must be at least 1 and `tolerance` above 0 and below 1; either may be left out to keep what the sketch has.
- The sketch keeps them (`fields.sketch.solver` in `pc.doc.feature`) and is solved again with them at once.

See also `pc.sketch.status`.

Example: Settings kept on the sketch.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 5}
pc.sketch.solver_settings{sketch = s, iterations = 500, tolerance = 1e-6}
local solver = pc.doc.feature{id = s}.fields.sketch.solver
assert(solver.max_iterations == 500 and solver.tolerance == 1e-6)
assert(pc.sketch.status{sketch = s}.solved)
```

`pc.sketch.repair`: Join ends of curves that nearly meet, and remove curves of no size, doubled curves and constraints left naming nothing.

- `tolerance` (number, optional): How near two ends must be to join, mm (0.01 when left out)
- `sketch` (id): The sketch to draw in
- Returns what was repaired, in words

Notes:

- It is the answer to a profile that fails with "profile is not closed" because ends miss by a hair, as a drawing brought in may.
- It answers in words, such as "2 end(s) joined, 1 duplicate curve(s) removed", or "nothing to repair".

See also `pc.sketch.import_dxf`, `pc.sketch.status`.

Example: Ends that miss by microns joined.

```lua
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
```

`pc.sketch.restore`: Put the sketch back as `data` holds it: an editing session cancelled.

- `data` (any): The sketch as doc.feature lists its data
- `sketch` (id): The sketch to draw in

Notes:

- `data` is the whole `fields` of `pc.doc.feature{id = s}`, plane included, taken before the edits; a table that is not a sketch's data is refused.

See also `pc.doc.feature`.

Example: Edits put back.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 5}
local saved = pc.doc.feature{id = s}.fields
pc.sketch.circle{sketch = s, x = 5, y = 2, radius = 1}
assert(#pc.sketch.geometry{sketch = s} == 10)
pc.sketch.restore{sketch = s, data = saved}
assert(#pc.sketch.geometry{sketch = s} == 8, "the circle and its centre are gone")
```

`pc.sketch.set_plane`: Move the sketch onto another plane, its geometry kept in its own coordinates.

- `sketch` (id): The sketch to draw in
- `normal` (list): The plane's normal, {x, y, z}
- `origin` (list, optional): Its origin, {x, y, z}
- `x_axis` (list, optional): The sketch's X direction, {x, y, z}

Notes:

- The geometry keeps its sketch coordinates and moves with the plane. Without `x_axis` the sketch's x is a direction square to the normal chosen for it (+Y for a normal along X), so give `x_axis` to know which way the geometry lies.
- The plane given is fixed: a sketch made on a datum stops following it, and `sketch.attachment` refuses it from then on.

See also `pc.sketch.attachment`, `pc.sketch.new`.

Example: A sketch moved onto a plane facing +X.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 4}
pc.sketch.set_plane{sketch = s, normal = {1, 0, 0}, origin = {5, 0, 0}, x_axis = {0, 1, 0}}
local pad = pc.design.pad{sketch = s, length = 2}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.min[1] - 5) < 1e-6 and math.abs(m.max[1] - 7) < 1e-6, "padded along +X from x = 5")
assert(math.abs(m.max[2] - 10) < 1e-6 and math.abs(m.max[3] - 4) < 1e-6, "sketch x along Y, y along Z")
```

`pc.sketch.array`: Repeat elements in rows and columns.

- `sketch` (id): The sketch to draw in
- `items` (list): The elements to repeat
- `rows` (integer)
- `cols` (integer)
- `dx` (number): The step between columns, mm
- `dy` (number): The step between rows, mm
- `linked` (boolean, optional): Copies stay the originals' size, spaced by one pitch along the rows and one down the columns (false)
- Returns {elements}: what it made

Notes:

- The items are the first copy: `rows = 2, cols = 3` makes five more. Columns step along the sketch's x by `dx`, rows along its y by `dy`.
- `rows` and `cols` must be at least 1, and one of them more than 1 ("an array needs elements and at least two rows or columns").
- Without `linked` the copies are free geometry; with it, constraints keep them the originals' size and on the pitch.
- It returns {elements, constraints}: the copies, their points and centres included.

See also `pc.sketch.draw`, `pc.design.linear_pattern`.

Example: A plate with six holes in two rows.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 50, height = 30}
local hole = pc.sketch.circle{sketch = s, x = 10, y = 10, radius = 2}
local made = pc.sketch.array{sketch = s, items = {hole}, rows = 2, cols = 3, dx = 15, dy = 10}
assert(#made.elements == 10, "five more circles and their centres: the original is the first")
local pad = pc.design.pad{sketch = s, length = 1}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.volume - (50 * 30 - 6 * math.pi * 4)) < 0.01, m.volume)
```

`pc.sketch.text`: Lay out text as closed outlines standing on a new point: the start of its first line on the baseline.

- `sketch` (id): The sketch to draw in
- `text` (string): What it says; a new line starts a line
- `at` (list): Where its point goes, {x, y}
- `font` (string, optional): IBM Plex Sans, IBM Plex Sans SemiBold, IBM Plex Mono, or a font file's path (IBM Plex Sans)
- `font_asset` (string, optional): The document asset a font file was kept as, which a recording names
- `size` (number, optional): The font's em, mm (10)
- `spacing` (number, optional): Added between letters, mm (0)
- `angle` (number, optional): Degrees it turns about its point (0)
- Returns {text, point}: the block and the point it stands on

Notes:

- A font file named by its path is kept in the document the first time it is used, so the text is made again from it on any machine; a font neither carried nor kept is refused with why.
- `at` is where the baseline starts: the letters stand on it, capitals reaching about 0.7 of `size`, the em.
- The letters are closed outlines that pad as they read; dragging the point returned moves the whole text.
- A `font` that is not one of the three bundled names is read as a file path, refused when no such file is there. Text with nothing to draw is refused.

See also `pc.sketch.text_edit`.

Example: Letters padded from the baseline.

```lua
local s = pc.sketch.new{plane = "XY"}
local t = pc.sketch.text{sketch = s, text = "PC", at = {0, 0}, size = 10}
assert(t.text and t.point)
local pad = pc.design.pad{sketch = s, length = 1}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.min[2]) < 0.2, "the baseline is at y = 0")
assert(m.max[2] > 6 and m.max[2] < 10, "capitals stand under the 10 mm em")
```

`pc.sketch.text_edit`: Change a text block, its outlines made again where its point stands.

- `sketch` (id): The sketch to draw in
- `block` (id): The text block, or its point
- `text` (string, optional)
- `font` (string, optional)
- `font_asset` (string, optional)
- `size` (number, optional): mm
- `spacing` (number, optional): mm
- `angle` (number, optional): degrees

Notes:

- `block` is either the `text` or the `point` that `sketch.text` returned.
- Only what is given changes, and the outlines are made again on the same point; a `size` of 0 or less is refused.

See also `pc.sketch.text`.

Example: A letter made twice as large.

```lua
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
```

`pc.sketch.to_bspline`: Make lines, arcs, circles, ellipses and conics into splines that are exactly them.

- `sketch` (id): The sketch to draw in
- `items` (list): The curves to make splines of
- Returns {elements}: what it made

Notes:

- Each curve is replaced and its id is gone; an arc's ends stay, as the spline's first and last control points. Arcs and circles become rational splines, exact.
- A spline is kind "other" in `pc.sketch.geometry`; its control points, degree, knots and weights are in `pc.doc.feature{id = s}.fields.sketch.geometry`.
- It returns {elements, constraints}: the new control points and the spline.

See also `pc.sketch.join`, `pc.sketch.spline_degree`.

Example: A circle made a spline pads the same disc.

```lua
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
```

`pc.sketch.spline_degree`: Raise or lower the degree of splines: raising keeps the curve, lowering fits the nearest one.

- `sketch` (id): The sketch to draw in
- `items` (list): The splines
- `by` (integer): 1 to raise, -1 to lower

Notes:

- Only the sign of `by` counts: any number above 0 raises one degree, any below lowers one, and 0 is refused.
- Raising adds a control point. The bspline tool draws degree 3, which the data leaves out: `degree` shows in it once changed.

See also `pc.sketch.spline_knots`, `pc.sketch.to_bspline`.

Example: A cubic raised to degree 4.

```lua
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
```

`pc.sketch.insert_knot`: Insert a knot into a spline where it passes nearest a point, the curve unchanged.

- `sketch` (id): The sketch to draw in
- `spline` (id): The spline
- `at` (list): A point near the curve, {x, y}

Notes:

- `at` need not lie on the curve: the knot goes at the parameter where the spline passes nearest it.
- Only a spline takes it; a line, arc or circle is refused ("is not a spline").

See also `pc.sketch.knot_multiplicity`, `pc.sketch.spline_knots`.

Example: A knot inserted halfway.

```lua
local s = pc.sketch.new{plane = "XY"}
local made = pc.sketch.draw{sketch = s, tool = "bspline", points = {{0, 0}, {10, 10}, {20, 0}, {30, 10}, "finish"}}
local spline = made.elements[#made.elements]
assert(#pc.sketch.spline_knots{sketch = s, spline = spline} == 0)
pc.sketch.insert_knot{sketch = s, spline = spline, at = {15, 5}}
local knots = pc.sketch.spline_knots{sketch = s, spline = spline}
assert(#knots == 1 and knots[1].multiplicity == 1)
assert(math.abs(knots[1].knot - 0.5) < 1e-6, "halfway along this symmetric spline")
```

`pc.sketch.knot_multiplicity`: Set how many times a spline's knot stands (1 up to the degree), or remove it with 0.

- `sketch` (id): The sketch to draw in
- `spline` (id): The spline
- `knot` (number): The knot's value, as sketch.spline_knots lists it
- `multiplicity` (integer)

Notes:

- `knot` is a value `sketch.spline_knots` lists; one where the spline has no knot is refused.
- A multiplicity above the degree is held at the degree. Asking for the one the knot has is refused ("the knot is unchanged").

See also `pc.sketch.spline_knots`, `pc.sketch.insert_knot`.

Example: A knot doubled, then removed.

```lua
local s = pc.sketch.new{plane = "XY"}
local made = pc.sketch.draw{sketch = s, tool = "bspline", points = {{0, 0}, {10, 10}, {20, 0}, {30, 10}, "finish"}}
local spline = made.elements[#made.elements]
pc.sketch.insert_knot{sketch = s, spline = spline, at = {15, 5}}
local knot = pc.sketch.spline_knots{sketch = s, spline = spline}[1].knot
pc.sketch.knot_multiplicity{sketch = s, spline = spline, knot = knot, multiplicity = 2}
assert(pc.sketch.spline_knots{sketch = s, spline = spline}[1].multiplicity == 2)
pc.sketch.knot_multiplicity{sketch = s, spline = spline, knot = knot, multiplicity = 0}
assert(#pc.sketch.spline_knots{sketch = s, spline = spline} == 0, "0 removes it")
```

`pc.sketch.spline_knots`: A spline's knots inside its ends and how many times each stands.

- `sketch` (id): The sketch to draw in
- `spline` (id): The spline
- Returns {{knot, multiplicity}}

Notes:

- The knots at the ends are not listed: a spline fresh from the bspline tool lists none.
- An element that is not a spline answers an empty list rather than an error.

See also `pc.sketch.insert_knot`, `pc.sketch.knot_multiplicity`.

Example: A half circle as a spline has one double knot.

```lua
local s = pc.sketch.new{plane = "XY"}
local arc = pc.sketch.arc{sketch = s, x = 0, y = 0, radius = 5, start = 0, ["end"] = 180}
local made = pc.sketch.to_bspline{sketch = s, items = {arc}}
local spline = made.elements[#made.elements]
local knots = pc.sketch.spline_knots{sketch = s, spline = spline}
assert(#knots == 1, "the knots at its ends are not listed")
assert(knots[1].knot == 0.5 and knots[1].multiplicity == 2)
```

`pc.sketch.spline_weight`: Weigh a spline's control point: more pulls the curve toward it.

- `sketch` (id): The sketch to draw in
- `spline` (id): The spline
- `point` (id): One of its control points
- `weight` (number): More than 0; 1 is plain

Notes:

- `point` is one of the spline's `control_points` in `pc.doc.feature`'s data, where its `weights` stand in the same order; any other point is refused.
- A weight of 0 or less is refused.

See also `pc.sketch.to_bspline`.

Example: A control point weighed three times.

```lua
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
```

`pc.sketch.join`: Merge curves that meet end to end into one B-spline following them.

- `sketch` (id): The sketch to draw in
- `items` (list): The lines, arcs, arcs of ellipses, parabolas and hyperbolas, and open splines to merge
- `tolerance` (number, optional): How far the spline may stray from the curves, mm (0.01)
- Returns {elements}: what it made

Notes:

- The curves are replaced by one spline ending where the chain ends; their ids are gone.
- It needs two or more curves meeting end to end in one chain: one curve, a gap or a branch is refused. A sharp corner, such as a rectangle's, fails at the default `tolerance` ("No spline follows these curves within 0.01 mm").

See also `pc.sketch.to_bspline`.

Example: A line and a quarter arc made one spline.

```lua
local s = pc.sketch.new{plane = "XY"}
local line = pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
local arc = pc.sketch.arc{sketch = s, x = 10, y = 5, radius = 5, start = -90, ["end"] = 0}
local made = pc.sketch.join{sketch = s, items = {line, arc}}
assert(#made.elements > 0)
local count = {}
for _, e in ipairs(pc.sketch.geometry{sketch = s}) do count[e.kind] = (count[e.kind] or 0) + 1 end
assert(count.line == nil and count.arc == nil, "both are replaced")
assert(count.other == 1, "by one spline")
```

`pc.sketch.set_constraint`: Make constraints driving or reference, active or not, parked or not.

- `sketch` (id): The sketch to draw in
- `items` (list): The constraints
- `driving` (boolean, optional): false: a reference dimension that only measures
- `active` (boolean, optional): false: kept but not solved
- `as` (string, optional): "radius" or "diameter": a circle's or arc's dimension measures that instead
- `parked` (boolean, optional): true: its symbol moves to the parked layer, drawn only while that layer shows; it still solves

Notes:

- `items` are constraint ids; an element's id among them is refused. Only the flags given change, and `driving = false` is refused for a constraint that is not a dimension.
- A dimension that is not driving measures and conflicts with nothing; a constraint that is not active is kept but left out of solving.
- `as` turns a radius into a diameter or back, keeping the constraint and the size it holds: its value doubles or halves, as does what a formula on it gives.

See also `pc.sketch.set_value`, `pc.sketch.status`.

Example: A repeated dimension made a reference.

```lua
local s = pc.sketch.new{plane = "XY"}
local sides = pc.sketch.rect{sketch = s, x = 0, y = 0, width = 30, height = 15}
pc.sketch.constrain{sketch = s, kind = "distance", items = {sides[1]}, value = 30}
local top = pc.sketch.constrain{sketch = s, kind = "distance", items = {sides[3]}}
assert(#pc.sketch.status{sketch = s}.redundant > 0, "the top's length says the bottom's again")
pc.sketch.set_constraint{sketch = s, items = top, driving = false}
assert(#pc.sketch.status{sketch = s}.redundant == 0, "a reference only measures")
```

`pc.sketch.mirror_sketch`: A new sketch on the same plane: this one's geometry mirrored across its Y axis.

- `sketch` (id): The sketch to draw in
- Returns the new sketch's id

Notes:

- It makes a new sketch, named after this one with "mirror", in the same body; this one is left as it is. The mirror is across the sketch's own Y axis: x becomes -x.
- For both halves in one profile, `sketch.merge` the two, or mirror within one sketch with `sketch.draw`'s mirror tool.

See also `pc.sketch.merge`, `pc.sketch.draw`.

Example: A rectangle mirrored across the Y axis.

```lua
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
```

`pc.sketch.merge`: A new sketch holding this one's geometry and other sketches', mapped onto its plane.

- `sketch` (id): The sketch to draw in
- `with` (list): The other sketches
- Returns the new sketch's id

Notes:

- It makes a new sketch, named after this one with "merged", on this one's plane and in its body; the sketches merged are left as they are. Constraints come with the geometry; under a turn or a mirror those stated against the sketch axes follow it (swapped on a quarter turn, made parallel or perpendicular to one another at any other angle). Projected geometry stays projected from its solid, sketch or datum.
- Each sketch in `with` must lie on a plane parallel to this one ("its plane is not parallel to this one"); its geometry is laid onto this plane, the distance between them dropped.

See also `pc.sketch.carbon_copy`, `pc.sketch.mirror_sketch`.

Example: Two sketches merged and padded as one.

```lua
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
```

`pc.sketch.carbon_copy`: Copy another sketch's geometry into this one, mapped onto its plane.

- `sketch` (id): The sketch to draw in
- `from` (id): The sketch to copy
- Returns {elements, constraints}: what it made

Notes:

- The geometry comes with its constraints, as geometry of this sketch: free to edit, not tied to the sketch it came from. Under a turn or a mirror the constraints stated against the sketch axes follow it, and projected geometry stays projected from its solid, sketch or datum.
- `from` must lie on a plane parallel to this one ("its plane is not parallel to this one"); a sketch cannot copy itself.

See also `pc.sketch.merge`, `pc.sketch.external_from`.

Example: A rectangle copied onto a sketch above it.

```lua
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
```

`pc.sketch.paste`: Add geometry held as a sketch of its own, moved by a step.

- `sketch` (id): The sketch to draw in
- `clipboard` (any): The geometry, as a sketch's fields (what copying in the sketcher holds)
- `by` (list): The step, {x, y}
- Returns {elements}: what it made

Notes:

- `clipboard` is a sketch's own data: `pc.doc.feature{id = s}.fields.sketch` of any sketch serves, and all of its geometry comes, with its constraints.
- `by` is a step {dx, dy} in mm. It returns {elements, constraints}.

See also `pc.sketch.carbon_copy`.

Example: A rectangle pasted beside itself.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 5}
local clip = pc.doc.feature{id = s}.fields.sketch
local made = pc.sketch.paste{sketch = s, clipboard = clip, by = {20, 0}}
assert(#made.elements == 8)
local pad = pc.design.pad{sketch = s, length = 1}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.volume - 100) < 1e-3 and math.abs(m.max[1] - 30) < 1e-6)
```

`pc.sketch.external`: Project edges of solids into the sketch as fixed references.

- `sketch` (id): The sketch to draw in
- `edges` (list): Each {body, point, direction}: a point on the edge and its direction, in the body's own frame
- `counts` (boolean, optional): true: it counts in the profile, as drawn geometry does; false (the default): it only guides the sketch
- Returns {elements}: what it made

Notes:

- Each edge is a point on it and its direction there, in the body's own frame; `pc.doc.edges` gives both (where the body sits, the same until it is moved). The body must be built first (`pc.doc.rebuild()`), else it is refused ("that body has no solid shape").
- It only guides unless `counts = true`; counted, the projected edges close a profile as drawn lines do. An edge square to the sketch plane projects to a point.
- It returns {elements, constraints}: the curves and the points at their ends, the curves marked `external` by `pc.sketch.geometry`.
- A point names an edge only within a tenth of the body's diagonal of it; one farther from every edge is refused ("no edge near ..."). An edge the sketch already holds comes once: picked again, at any point along it, it is passed over, and a call that brings nothing new is refused ("... already in the sketch ...").

See also `pc.sketch.intersection`, `pc.doc.edges`, `pc.sketch.external_defining`.

Example: A block's top edges projected and padded higher.

```lua
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
```

`pc.sketch.intersection`: Add where faces of solids cross the sketch plane, as fixed references.

- `sketch` (id): The sketch to draw in
- `faces` (list): Each {body, point, normal}: a point on the face and its normal there, in the body's own frame
- `counts` (boolean, optional): true: it counts in the profile, as drawn geometry does; false (the default): it only guides the sketch
- Returns {elements}: what it made

Notes:

- Each face is a point on it and its outward normal there, in the body's own frame; `pc.doc.faces` gives both. The body must be built first.
- A face the sketch plane does not cross adds nothing. What it adds only guides unless `counts = true`.
- It returns {elements, constraints}: the curves and the points at their ends.

See also `pc.sketch.external`, `pc.doc.faces`.

Example: Where a block's top face crosses a sketch through its middle.

```lua
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
```

`pc.sketch.constraints`: List the sketch's constraints.

- `sketch` (id): The sketch to draw in
- Returns a list of {id, kind, items, value?}

Notes:

- `kind` is the stored name (Coincident, Horizontal, Length, Diameter, Angle, ...): "distance" on a line lists as Length. `value` is a dimension's, angles in degrees, as its formula sets it when it has one.
- `items` are the element ids it ties. The origin and the axes show as fixed ids ending in 0001 (origin), 0002 (x axis) and 0003 (y axis), not by name.

See also `pc.sketch.status`, `pc.sketch.set_constraint`.

Example: An angle listed in degrees.

```lua
local s = pc.sketch.new{plane = "XY"}
local a = pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
local b = pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 10, y2 = 10}
local made = pc.sketch.constrain{sketch = s, kind = "angle", items = {a, b}}
local listed = pc.sketch.constraints{sketch = s}
assert(#listed == 1 and listed[1].id == made[1])
assert(listed[1].kind == "Angle" and math.abs(listed[1].value - 45) < 1e-4, "degrees")
assert(listed[1].items[1] == a and listed[1].items[2] == b)
```

`pc.sketch.wall_thickness`: How thin the sketch's closed profile gets, for printing.

- `sketch` (id): The sketch to draw in
- `minimum` (number, optional): The thinnest wall that prints, mm; the Sketcher preference when left out
- Returns {thinnest, where = {x, y}, minimum, thin, regions}: the thinnest wall in mm, where it is, whether it is under the minimum, and each region's own

Notes:

- It reads the sketch's closed profile, and refuses a sketch with none ("profile is not closed").
- `minimum` is the Sketcher preference when left out (0.8 mm unless changed); `thin` is true when the thinnest wall is under it.

See also `pc.sketch.status`.

Example: A frame with 1 mm walls.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
pc.sketch.rect{sketch = s, x = 1, y = 1, width = 18, height = 8}
local wall = pc.sketch.wall_thickness{sketch = s, minimum = 2.5}
assert(math.abs(wall.thinnest - 1) < 1e-4, "the frame's wall is 1 mm")
assert(wall.thin and wall.minimum == 2.5)
assert(not pc.sketch.wall_thickness{sketch = s, minimum = 0.5}.thin)
```

`pc.sketch.status`: How constrained the sketch is, and what conflicts.

- `sketch` (id): The sketch to draw in
- Returns {dof, solved, redundant, conflicting}

Notes:

- `dof` is the freedom left: 0 is fully constrained. A free point has 2, a free circle 3 (its centre and radius); a line's freedom is its two end points'.
- `solved` false with ids in `conflicting` means constraints contradict; every constraint taking part is listed, not only the newest. `redundant` lists those that say again what others say; the sketch still solves. While `solved` is false, what is built from the sketch fails at `pc.doc.rebuild()`.

See also `pc.sketch.constrain`, `pc.sketch.constraints`.

Example: A circle constrained, then over-constrained.

```lua
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
```

`pc.sketch.delete`: Delete elements or constraints, and what depends on them.

- `sketch` (id): The sketch to draw in
- `items` (list): Element or constraint ids

Notes:

- Deleting a point takes the curves that end or centre on it and their constraints. Deleting a curve leaves its end points behind as loose points.
- A constraint's id deletes only that constraint. The origin and the axes are passed over; an id the sketch does not have is refused.

See also `pc.sketch.construction`, `pc.sketch.repair`.

Example: A corner deleted with the two lines on it.

```lua
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
```

`pc.sketch.construction`: Make elements construction geometry, or normal again.

- `sketch` (id): The sketch to draw in
- `items` (list): Element ids
- `on` (boolean, optional): true (the default) or false

Notes:

- Construction geometry is left out of profiles: a construction circle inside an outline cuts no hole. Constraints on it still hold.
- `sketch.draw` makes construction geometry from the start with `construction = true`.

See also `pc.sketch.draw`, `pc.sketch.delete`.

Example: A circle that guides, then cuts.

```lua
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
```

`pc.sketch.internal_geometry`: Show or hide curves' internal geometry: an ellipse's axes and foci, a parabola's or hyperbola's axis and focus, a B-spline's control polygon, as construction held to its curve.

- `sketch` (id): The sketch to draw in
- `items` (list): The curves, or pieces of their internal geometry
- `show` (boolean, optional): true makes what is missing, false takes away the pieces nothing else holds; left out, it shows when a piece is missing and hides otherwise
- Returns {shown, elements}: whether it showed, and what it made or took away

Notes:

- Without `show` it switches: the same call twice makes the pieces and takes them away again. Give `show = true` to be sure they are there.
- An ellipse gets its major and minor axes with their ends and its two foci, all construction held to it. Items naming no ellipse, parabola, hyperbola or B-spline are refused.

See also `pc.sketch.draw`, `pc.sketch.constrain`.

Example: An ellipse's axes and foci shown and hidden.

```lua
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
```

`pc.sketch.section_view`: Cut away everything on the viewer's side of the sketch plane while it is edited.

- `sketch` (id): The sketch to draw in
- `on` (boolean, optional): true (the default) or false

Notes:

- A view setting only: nothing built from the sketch changes, and it shows only while the sketch is open for editing in the window.
- The sketch's data carries `section_view = true` while it is on.

Example: The setting kept on the sketch.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.section_view{sketch = s}
assert(pc.doc.feature{id = s}.fields.section_view == true)
pc.sketch.section_view{sketch = s, on = false}
assert(pc.doc.feature{id = s}.fields.section_view == nil)
```

`pc.sketch.remove_axis_alignment`: Turn the horizontal and vertical constraints of lines into parallel and perpendicular ones among them, so the group keeps its shape and turns as a whole.

- `sketch` (id): The sketch to draw in
- `items` (list): The lines
- Returns how many constraints changed

Notes:

- It returns how many horizontal and vertical constraints went; one fewer parallel or perpendicular constraint takes their place, so the group gains the freedom to turn. Lines with none answer 0.

See also `pc.sketch.constrain`, `pc.sketch.status`.

Example: A rectangle freed to turn.

```lua
local s = pc.sketch.new{plane = "XY"}
local sides = pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
assert(pc.sketch.status{sketch = s}.dof == 4)
assert(pc.sketch.remove_axis_alignment{sketch = s, items = sides} == 4)
for _, c in ipairs(pc.sketch.constraints{sketch = s}) do
  assert(c.kind == "Parallel" or c.kind == "Perpendicular", c.kind)
end
assert(pc.sketch.status{sketch = s}.dof == 5, "still a rectangle, free to turn")
```

`pc.sketch.generator`: Change the numbers a generated sketch (a gear, a sprocket, a shaft, a keyway) is made from, or detach it into a plain sketch.

- `sketch` (id): The generated sketch
- `detach` (boolean, optional): Keep the curves as they are and forget the numbers: a plain sketch to edit by hand
- Other arguments: The numbers to change, such as teeth = 24 or module = 1.5; a shaft takes sections = {{length = 20, diameter = 10, chamfer = 0.5, fillet = 0}, ...}
- Returns what the numbers come to: its diameters, or its length

Notes:

- Called with only `sketch` it changes nothing and answers the sizes: a gear's base, pitch, root and tip diameters, a sprocket's pitch, root and tip, a shaft's length.
- A field the generator lacks is refused, naming the ones it has. The sketch's curves are made again from the numbers, so anything drawn in it by hand goes.
- `detach = true` keeps the curves and drops the numbers for good. Each number is also a parameter by its name (`module`, `teeth`), which `doc.set_formula` binds to a formula.

See also `pc.design.gear`, `pc.design.sprocket`, `pc.design.shaft`, `pc.doc.set_formula`.

Example: A gear's teeth changed, and the tip diameter with them.

```lua
local gear = pc.design.gear{module = 1.5, teeth = 20}
assert(pc.sketch.generator{sketch = gear}.tip_diameter == 1.5 * 22)
local size = pc.sketch.generator{sketch = gear, teeth = 30}
assert(size.pitch_diameter == 1.5 * 30 and size.tip_diameter == 1.5 * 32)
pc.sketch.generator{sketch = gear, detach = true}
assert(pc.doc.feature{id = gear}.fields.generator == nil, "a plain sketch")
```

### design

`pc.design.pad`: Pad a sketch.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- `length` is 10 mm when left out. The pad grows along the sketch's normal; `reversed = true` grows it the other way, `symmetric = true` half each way.
- It goes in the sketch's body. With no sketch, `face_point` and `face_normal` name a flat face of the solid to extrude instead.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.sketch.new`, `pc.design.pocket`.

Example: A plate padded from a rectangle.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 40, height = 30}
local pad = pc.design.pad{sketch = s, length = 5}
assert(#pc.doc.rebuild() == 0, "the pad builds")
local body = pc.doc.feature{id = pad}.body
assert(math.abs(pc.doc.measure{body = body}.volume - 40 * 30 * 5) < 1e-3)
assert(#pc.doc.faces{body = body} == 6, "a box has six faces")
```

`pc.design.pocket`: Cut a sketch into the body.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- It cuts against its sketch's normal: from a sketch at the top face's height it digs down into the solid. From a sketch on the bottom (XY at 0 under a pad) it cuts away from the material and removes nothing, without an error; `reversed = true` turns it.
- `through_all = true` cuts through everything; else `depth`, 5 mm when left out.
- It is refused in a body with no solid feature yet. Its sketch must be in the padded body: `pc.sketch.new{body = ...}`, since a sketch made without `body` starts a new one.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.sketch.new`, `pc.design.hole`.

Example: A square window cut through a plate.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 40, height = 30}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
local top = pc.sketch.new{body = body, plane = "XY", offset = 5}
pc.sketch.rect{sketch = top, x = 15, y = 10, width = 10, height = 10}
pc.design.pocket{sketch = top, through_all = true}
assert(#pc.doc.rebuild() == 0, "the pocket builds")
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (40 * 30 - 10 * 10) * 5) < 1e-3, volume)
assert(#pc.doc.faces{body = body} == 10, "six faces and four walls")
```

`pc.design.revolve`: Turn a sketch about an axis.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- It turns the sketch about the sketch's own y axis through its origin (`axis = "SketchY"`; on an XZ sketch that is the world's Z), 360 degrees when `angle_deg` is left out; `axis = "SketchX"` or `axis = {Custom = {origin = {x, y}, dir = {x, y}}}` in sketch coordinates turn it about another.
- A profile that touches the axis makes a solid of revolution; one that crosses it fails at `pc.doc.rebuild()` (the profile sits on the axis).
- It is refused without a sketch.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.design.groove`, `pc.design.helix`.

Example: A tube turned from a rectangle beside the axis.

```lua
local s = pc.sketch.new{plane = "XZ"}
pc.sketch.rect{sketch = s, x = 10, y = 0, width = 5, height = 20}
local tube = pc.design.revolve{sketch = s}
assert(#pc.doc.rebuild() == 0, "the revolution builds")
local body = pc.doc.feature{id = tube}.body
-- The sketch's y axis on XZ is the world's Z: a tube 10 to 15 mm out, 20 tall.
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - math.pi * (15 ^ 2 - 10 ^ 2) * 20) < 1e-3, volume)
```

`pc.design.groove`: Cut a sketch turned about an axis.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- It takes away what `design.revolve` would add: turned about the y axis through the origin, 360 degrees; `axis` and `angle_deg` are the same fields.
- It is refused where no solid is built yet, and its profile must be drawn in the same history as that solid.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.design.revolve`, `pc.design.pocket`.

Example: A ring groove cut into a turned shaft.

```lua
local s = pc.sketch.new{plane = "XZ"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 20}
local shaft = pc.design.revolve{sketch = s}
local body = pc.doc.feature{id = shaft}.body
local ring = pc.sketch.new{body = body, plane = "XZ"}
pc.sketch.rect{sketch = ring, x = 8, y = 8, width = 2, height = 4}
pc.design.groove{sketch = ring}
assert(#pc.doc.rebuild() == 0, "the groove builds")
-- A 4 mm wide ring cut 2 mm deep into a 10 mm radius shaft.
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (math.pi * 100 * 20 - math.pi * (100 - 64) * 4)) < 1e-3, volume)
```

`pc.design.loft`: Loft through sketches.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- `sections = {a, b, ...}` lists the sketches in order; given only `sketch`, the loft has that one section and fails at `pc.doc.rebuild()` (a loft needs at least two sections).
- It goes in the first section's body; later sections may be sketches of other bodies. A sketch of a single point as the first or last section closes the loft to a tip.
- `ruled = true` joins the sections with straight walls; else the walls run smoothly through them. `closed = true` loops back to the first.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.design.subtractive_loft`, `pc.design.pipe`.

Example: A square frustum lofted between two rectangles.

```lua
local base = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = base, x = -10, y = -10, width = 20, height = 20}
local body = pc.doc.feature{id = base}.body
local top = pc.sketch.new{body = body, plane = "XY", offset = 15}
pc.sketch.rect{sketch = top, x = -5, y = -5, width = 10, height = 10}
pc.design.loft{sections = {base, top}, ruled = true}
assert(#pc.doc.rebuild() == 0, "the loft builds")
-- A frustum of a square pyramid: h / 3 * (A1 + A2 + sqrt(A1 * A2)).
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - 15 / 3 * (400 + 100 + 200)) < 1e-3, volume)
assert(#pc.doc.faces{body = body} == 6)
```

`pc.design.subtractive_loft`: Cut a loft through sketches.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- `sections = {a, b, ...}` lists the sketches in order; given only `sketch`, the loft has that one section and fails at `pc.doc.rebuild()` (a loft needs at least two sections).
- It goes in the first section's body; later sections may be sketches of other bodies. A sketch of a single point as the first or last section closes the loft to a tip.
- `ruled = true` joins the sections with straight walls; else the walls run smoothly through them. `closed = true` loops back to the first.
- It is refused in a body with no solid feature yet.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.design.loft`.

Example: A conical dimple: a circle lofted down to a point and cut.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = -10, y = -10, width = 20, height = 20}
local pad = pc.design.pad{sketch = s, length = 10}
local body = pc.doc.feature{id = pad}.body
local mouth = pc.sketch.new{body = body, plane = "XY", offset = 10}
pc.sketch.circle{sketch = mouth, x = 0, y = 0, radius = 5}
local tip = pc.sketch.new{body = body, plane = "XY", offset = 4}
pc.sketch.point{sketch = tip, x = 0, y = 0}
pc.design.subtractive_loft{sections = {mouth, tip}}
assert(#pc.doc.rebuild() == 0, "the cut builds")
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (4000 - math.pi * 25 * 6 / 3)) < 1e-3, volume)
```

`pc.design.pipe`: Sweep a sketch along a path.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- `sketch` is the profile. The path is `spine`, a sketch id; left out, it is the latest other sketch of the profile's body. The profile sits at either end of the path, and the pipe runs from there.
- `spine` may be a sketch of any body. Left out, it is refused when the profile's body has no other sketch.
- A path's sharp corners are mitred (`corner = "Transformed"`); `orientation` sets how the profile turns along it.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.design.subtractive_pipe`, `pc.sketch.polyline`.

Example: A rod swept up and across along a bent path.

```lua
local profile = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = profile, x = 0, y = 0, radius = 2}
local body = pc.doc.feature{id = profile}.body
local path = pc.sketch.new{body = body, plane = "XZ"}
pc.sketch.polyline{sketch = path, points = {{0, 0}, {0, 20}, {15, 20}}}
-- Up 20 mm and across 15: the path is the body's latest other sketch.
local pipe = pc.design.pipe{sketch = profile}
assert(#pc.doc.rebuild() == 0, "the pipe builds")
assert(pc.doc.feature{id = pipe}.fields.Pipe.spine == path)
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - math.pi * 4 * 35) < 1e-3, volume)
```

`pc.design.subtractive_pipe`: Cut a sketch swept along a path.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- `sketch` is the profile. The path is `spine`, a sketch id; left out, it is the latest other sketch of the profile's body. The profile sits at either end of the path, and the pipe runs from there.
- `spine` may be a sketch of any body. Left out, it is refused when the profile's body has no other sketch.
- A path's sharp corners are mitred (`corner = "Transformed"`); `orientation` sets how the profile turns along it.
- It is refused in a body with no solid feature yet.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.design.pipe`.

Example: A half-round channel cut along the top of a block.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 10}
local body = pc.doc.feature{id = pad}.body
local profile = pc.sketch.new{body = body, plane = "YZ"}
pc.sketch.circle{sketch = profile, x = 5, y = 10, radius = 2}
local path = pc.sketch.new{body = body, plane = "XZ", offset = -5}
pc.sketch.line{sketch = path, x1 = 0, y1 = 10, x2 = 20, y2 = 10}
pc.design.subtractive_pipe{sketch = profile, spine = path}
assert(#pc.doc.rebuild() == 0, "the channel builds")
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (2000 - math.pi * 4 * 20 / 2)) < 1e-3, volume)
```

`pc.design.helix`: Sweep a sketch along a helix.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- It sweeps the sketch along a helix about the sketch's y axis through its origin (`axis`, as `design.revolve` takes it). `mode` says which two of `pitch`, `height` and `turns` count: PitchHeight (the default: 5 mm pitch, 20 mm high; `turns` is then ignored), PitchTurns, HeightTurns, or HeightTurnsGrowth with `growth` per turn.
- `left_handed`, `cone_angle_deg` and `reversed` are fields too.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.design.subtractive_helix`, `pc.design.revolve`.

Example: A square section swept a quarter turn.

```lua
local s = pc.sketch.new{plane = "XZ"}
pc.sketch.rect{sketch = s, x = 9, y = 0, width = 2, height = 2}
local coil = pc.design.helix{sketch = s, mode = "PitchTurns", pitch = 4, turns = 0.25}
assert(#pc.doc.rebuild() == 0, "the helix builds")
local body = pc.doc.feature{id = coil}.body
-- A 2 x 2 square whose centre runs a quarter turn at radius 10.
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - 4 * 2 * math.pi * 10 / 4) < 1e-3, volume)
```

`pc.design.subtractive_helix`: Cut a sketch swept along a helix.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- It sweeps the sketch along a helix about the sketch's y axis through its origin (`axis`, as `design.revolve` takes it). `mode` says which two of `pitch`, `height` and `turns` count: PitchHeight (the default: 5 mm pitch, 20 mm high; `turns` is then ignored), PitchTurns, HeightTurns, or HeightTurnsGrowth with `growth` per turn.
- `left_handed`, `cone_angle_deg` and `reversed` are fields too.
- It is refused in a body with no solid feature yet. `keep_inside = true` keeps what the sweep shares with the body instead of cutting it.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.design.helix`, `pc.design.hole`.

Example: A thread-like groove cut a quarter turn into a cylinder.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 10}
local pad = pc.design.pad{sketch = s, length = 10}
local body = pc.doc.feature{id = pad}.body
local groove = pc.sketch.new{body = body, plane = "XZ"}
pc.sketch.rect{sketch = groove, x = 9, y = 4, width = 2, height = 2}
pc.design.subtractive_helix{sketch = groove, mode = "PitchTurns", pitch = 4, turns = 0.25}
assert(#pc.doc.rebuild() == 0, "the cut builds")
-- Only the half of the square inside the cylinder (1 x 2, centre at radius 9.5) is cut.
local cut = 2 * 2 * math.pi * 9.5 / 4
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (math.pi * 100 * 10 - cut)) < 1e-2, volume)
```

`pc.design.primitive`: Add a box, cylinder, sphere, cone, torus, ellipsoid, prism or wedge.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- `variant` (string, optional): box (the default), cylinder, sphere, cone, torus, ellipsoid, prism or wedge
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- It takes no sketch, so `body` is required. Left as it comes, a shape is about 10 mm across and placed at the body's origin: a box from 0 to 10 along each axis, a cylinder of radius 5 standing 10 tall on it. A `variant` other than the shapes listed is refused.
- `kind` replaces the shape whole: `kind = {Cylinder = {radius = 3, height = 8, angle_deg = 360}}`, every field of it given, else it is refused (missing field). `pc.doc.feature{id = ...}` shows a shape's fields.
- `placement = {origin = {x, y, z}, x_axis = {..}, z_axis = {..}}` places it in the body's frame; an `x_axis` along the `z_axis` fails at `pc.doc.rebuild()`.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.design.subtractive_primitive`, `pc.doc.new_body`.

Example: A box with a cylinder standing on it.

```lua
local body = pc.doc.new_body{}
pc.design.primitive{body = body, kind = {Box = {length = 20, width = 10, height = 5}}}
pc.design.primitive{body = body, variant = "cylinder",
  kind = {Cylinder = {radius = 2, height = 6, angle_deg = 360}},
  placement = {origin = {10, 5, 5}, x_axis = {1, 0, 0}, z_axis = {0, 0, 1}}}
assert(#pc.doc.rebuild() == 0, "the box and the post build")
local m = pc.doc.measure{body = body}
assert(math.abs(m.volume - (1000 + math.pi * 4 * 6)) < 1e-3, m.volume)
assert(math.abs(m.max[3] - 11) < 1e-6, "the post stands on the box")
```

`pc.design.subtractive_primitive`: Cut a box, cylinder, sphere, cone, torus, ellipsoid, prism or wedge.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- `variant` (string, optional): box (the default), cylinder, sphere, cone, torus, ellipsoid, prism or wedge
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- It takes no sketch, so `body` is required. Left as it comes, a shape is about 10 mm across and placed at the body's origin: a box from 0 to 10 along each axis, a cylinder of radius 5 standing 10 tall on it. A `variant` other than the shapes listed is refused.
- `kind` replaces the shape whole: `kind = {Cylinder = {radius = 3, height = 8, angle_deg = 360}}`, every field of it given, else it is refused (missing field). `pc.doc.feature{id = ...}` shows a shape's fields.
- `placement = {origin = {x, y, z}, x_axis = {..}, z_axis = {..}}` places it in the body's frame; an `x_axis` along the `z_axis` fails at `pc.doc.rebuild()`.
- It is refused in a body with no solid feature yet.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.design.primitive`, `pc.design.hole`.

Example: A cylinder cut through the default box.

```lua
local body = pc.doc.new_body{}
pc.design.primitive{body = body}
pc.design.subtractive_primitive{body = body, variant = "cylinder",
  kind = {Cylinder = {radius = 3, height = 10, angle_deg = 360}},
  placement = {origin = {5, 5, 0}, x_axis = {1, 0, 0}, z_axis = {0, 0, 1}}}
assert(#pc.doc.rebuild() == 0, "the box and the bore build")
-- The default box is 10 mm each way from the origin.
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (1000 - math.pi * 9 * 10)) < 1e-3, volume)
```

`pc.design.hole`: Drill holes at a sketch's circles and points.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- It drills at every circle's centre and every point of its sketch; the circles' sizes are ignored, the hole's own `diameter` (5 mm when left out) is what it drills.
- It drills against the sketch's normal, `depth` deep (10 mm) or `through_all = true`. A sketch on the bottom of a pad drills away from the material and removes nothing, without an error; `reversed = true` turns it.
- Counterbores, countersinks and threads are fields too (`cut`, `threaded`, `thread`); docs/HOLES.md describes them, and `pc.doc.feature{id = ...}` shows a hole's fields.
- `nut_trap = true` cuts a hexagonal pocket for a captive nut at the hole's mouth, sized from the ISO 4032 nut of its metric `thread` with 0.3 mm clearance; give a table for more: `{side = "Bottom", clearance, depth, across_flats, turn_deg, standard = "Din934"}`. At the bottom it ends where a hole with a `depth` ends; through all, it is refused.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.sketch.circle`, `pc.design.pocket`.

Example: An M3 clearance hole with a nut trap at its mouth.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 30, height = 20}
local body = pc.doc.feature{id = pc.design.pad{sketch = s, length = 6}}.body
local at = pc.sketch.new{body = body, plane = "XY", offset = 6}
pc.sketch.point{sketch = at, x = 15, y = 10}
pc.design.hole{sketch = at, thread = "M3", through_all = true, nut_trap = true}
assert(#pc.doc.rebuild() == 0, "the hole and its trap build")
-- The nut, 5.5 across the flats and 2.4 thick, each with 0.3 more.
local hex = math.sqrt(3) / 2 * 5.8 ^ 2
local bore = math.pi * 1.7 ^ 2
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (30 * 20 * 6 - bore * 3.3 - hex * 2.7)) < 1e-3, volume)
```

`pc.design.fillet`: Round edges.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- Without `edges` it rounds every edge of the solid (`edges = "All"`); `radius` is 1 mm when left out. In the app, edges selected in the view are taken instead.
- `edges = {Edges = {{point = {x, y, z}, direction = {x, y, z}}, ...}}` picks edges by a point on each and its direction there, and `edges = {Faces = {{point = .., normal = ..}}}` every edge around those faces, in the body's own frame. A bare list of picks is refused.
- Edges running on tangentially are taken too (`follow_tangent`, on by default).
- A radius larger than the faces beside an edge can take fails at `pc.doc.rebuild()`.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.doc.faces`, `pc.design.chamfer`.

Example: One top edge of a block rounded.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 40, height = 30}
local pad = pc.design.pad{sketch = s, length = 10}
local body = pc.doc.feature{id = pad}.body
pc.design.fillet{body = body, radius = 3,
  edges = {Edges = {{point = {20, 0, 10}, direction = {1, 0, 0}}}}}
assert(#pc.doc.rebuild() == 0, "the fillet builds")
assert(#pc.doc.faces{body = body} == 7, "six faces and the round")
local taken = (9 - math.pi * 9 / 4) * 40
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (12000 - taken)) < 0.01, volume)
```

`pc.design.chamfer`: Bevel edges.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- Without `edges` it bevels every edge of the solid (`edges = "All"`); `size` is 1 mm when left out. In the app, edges selected in the view are taken instead.
- `edges = {Edges = {{point = {x, y, z}, direction = {x, y, z}}, ...}}` picks edges by a point on each and its direction there, and `edges = {Faces = {{point = .., normal = ..}}}` every edge around those faces, in the body's own frame. A bare list of picks is refused.
- Edges running on tangentially are taken too (`follow_tangent`, on by default).
- `mode` is EqualDistance, TwoDistances (with `size2`) or DistanceAngle (with `angle_deg`).

See also `pc.doc.rebuild`, `pc.design.set`, `pc.doc.faces`, `pc.design.fillet`.

Example: The top face's edges bevelled.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 40, height = 30}
local pad = pc.design.pad{sketch = s, length = 10}
local body = pc.doc.feature{id = pad}.body
pc.design.chamfer{body = body, size = 2,
  edges = {Faces = {{point = {20, 15, 10}, normal = {0, 0, 1}}}}}
assert(#pc.doc.rebuild() == 0, "the chamfer builds")
assert(#pc.doc.faces{body = body} == 10, "six faces and four bevels")
-- A 2 x 2 prism along each edge, less what two share at a corner.
local taken = 2 * (40 + 30) * 2 - 4 * 8 / 3
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (12000 - taken)) < 0.01, volume)
```

`pc.design.draft`: Tilt faces.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- `face_point` and `face_normal` name the neutral plane's face and are required; `faces = {{point = .., normal = ..}, ...}` are the faces to tilt. Without `faces` it fails at `pc.doc.rebuild()` (no faces to tilt).
- `angle_deg` is 1.5 when left out. The faces keep their place on the neutral plane and lean outward away from it, against its outward normal; `reversed = true` leans them in.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.doc.faces`, `pc.design.thickness`.

Example: One side of a block drafted 10 degrees from its bottom.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 10}
local body = pc.doc.feature{id = pad}.body
-- The bottom is the neutral plane; the right side leans out 10 degrees above it.
pc.design.draft{body = body, angle_deg = 10,
  face_point = {10, 5, 0}, face_normal = {0, 0, -1},
  faces = {{point = {20, 5, 5}, normal = {1, 0, 0}}}}
assert(#pc.doc.rebuild() == 0, "the draft builds")
local lean = 10 * math.tan(math.rad(10))
local m = pc.doc.measure{body = body}
assert(math.abs(m.max[1] - (20 + lean)) < 1e-4, m.max[1])
assert(math.abs(m.volume - (2000 + lean * 10 / 2 * 10)) < 1e-3, m.volume)
```

`pc.design.thickness`: Hollow the solid.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- `face_point` and `face_normal` name the face it opens and are required; `faces = {{point = .., normal = ..}, ...}` opens several, replacing that one.
- `value` is the wall, 1 mm when left out, inside the solid; `inward = false` puts the walls outside it, `both_sides = true` on both.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.doc.faces`, `pc.design.draft`.

Example: A block hollowed into an open box.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 10}
local body = pc.doc.feature{id = pad}.body
-- An open box: the top face taken away, 1 mm walls left inside.
pc.design.thickness{body = body, face_point = {10, 5, 10}, face_normal = {0, 0, 1}}
assert(#pc.doc.rebuild() == 0, "the shell builds")
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (2000 - 18 * 8 * 9)) < 1e-3, volume)
```

`pc.design.delete_faces`: Delete faces and close the openings from their neighbours.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- `face_point` and `face_normal` name a face and are required; `faces = {{point = .., normal = ..}, ...}` deletes several, replacing that one. A bore's wall faces its axis: its normal points inward.
- Faces whose removal leaves a neighbour with nothing around it (a blind hole's wall without its bottom) fail at `pc.doc.rebuild()`; delete them together.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.doc.faces`, `pc.design.recognize_holes`.

Example: A through hole closed by deleting its wall.

```lua
local body = pc.doc.new_body{}
pc.design.primitive{body = body, kind = {Box = {length = 20, width = 10, height = 5}}}
local at = pc.sketch.new{body = body, plane = "XY", offset = 5}
pc.sketch.circle{sketch = at, x = 10, y = 5, radius = 2}
pc.design.pocket{sketch = at, through_all = true}
-- The bore's wall, picked at x = 12 where it faces the axis: the hole closes.
pc.design.delete_faces{body = body, face_point = {12, 5, 2.5}, face_normal = {-1, 0, 0}}
assert(#pc.doc.rebuild() == 0, "the deletion builds")
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - 1000) < 1e-3, volume)
assert(#pc.doc.faces{body = body} == 6)
```

`pc.design.offset_faces`: Push or pull faces along their normals, their neighbours following.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- `face_point` and `face_normal` name a face and are required; `faces` lists several, replacing that one.
- `distance` (1 mm when left out) moves the faces along their outward normals. A bore's outward normal points at its axis, so a positive distance makes a hole smaller and a negative one wider.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.design.move_faces`, `pc.doc.faces`.

Example: A hole widened by 1 mm all round.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
local at = pc.sketch.new{body = body, plane = "XY", offset = 5}
pc.sketch.circle{sketch = at, x = 10, y = 5, radius = 2}
pc.design.pocket{sketch = at, through_all = true}
-- A bore's outward normal points at its axis: a negative distance widens it.
pc.design.offset_faces{body = body, distance = -1,
  face_point = {12, 5, 2.5}, face_normal = {-1, 0, 0}}
assert(#pc.doc.rebuild() == 0, "the offset builds")
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (1000 - math.pi * 9 * 5)) < 1e-3, volume)
```

`pc.design.move_faces`: Move or turn faces, their neighbours following.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- `face_point` and `face_normal` name a face and are required; `faces` lists several, replacing that one.
- `translation` is 1 mm out along the face's normal when left out; give `translation = {0, 0, 0}` for a turn alone. `angle_deg` turns the faces about the line through `axis_point` (the face's point) along `axis_dir` (Z), in the body's own frame.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.design.offset_faces`, `pc.doc.faces`.

Example: A block's top face tilted about its middle.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 10}
local body = pc.doc.feature{id = pad}.body
-- The top face turned 10 degrees about a line across its middle.
pc.design.move_faces{body = body, face_point = {10, 5, 10}, face_normal = {0, 0, 1},
  translation = {0, 0, 0}, angle_deg = 10, axis_point = {10, 0, 10}, axis_dir = {0, 1, 0}}
assert(#pc.doc.rebuild() == 0, "the move builds")
local m = pc.doc.measure{body = body}
assert(math.abs(m.volume - 2000) < 1e-3, "what one end gains the other loses")
assert(math.abs(m.max[3] - (10 + 10 * math.tan(math.rad(10)))) < 1e-4, m.max[3])
```

`pc.design.mirror`: Mirror the last feature.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- `originals = {id, ...}` are the features it repeats; left out, it takes the body's last feature that is not a dress-up, a pattern, a mirror or a boolean. It is refused in a body with no solid feature yet.
- A copy that lands outside the solid (a pocket repeated past the material) changes nothing, without an error.
- The plane is YZ through the body's origin when left out; `plane = "XY"` or `"XZ"` takes another, and `face_point` with `face_normal` a flat face of the solid.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.design.linear_pattern`.

Example: A post mirrored across the YZ plane.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = -10, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
local corner = pc.sketch.new{body = body, plane = "XY", offset = 5}
pc.sketch.rect{sketch = corner, x = 5, y = 0, width = 3, height = 3}
local post = pc.design.pad{sketch = corner, length = 4}
pc.design.mirror{body = body, originals = {post}, plane = "YZ"}
assert(#pc.doc.rebuild() == 0, "the mirror builds")
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (1000 + 2 * 36)) < 1e-6, volume)
```

`pc.design.linear_pattern`: Repeat the last feature along a line.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- `originals = {id, ...}` are the features it repeats; left out, it takes the body's last feature that is not a dress-up, a pattern, a mirror or a boolean. It is refused in a body with no solid feature yet.
- A copy that lands outside the solid (a pocket repeated past the material) changes nothing, without an error.
- `axis` is X when left out (Y, Z, or `{Custom = {origin = {x, y, z}, dir = {x, y, z}}}`); `occurrences` (3) counts the original; `length` (30 mm) runs from the first to the last, or is the gap between them with `spacing_mode = true`.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.design.polar_pattern`.

Example: A hole repeated three times along X.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 30, height = 10}
local pad = pc.design.pad{sketch = s, length = 4}
local body = pc.doc.feature{id = pad}.body
local at = pc.sketch.new{body = body, plane = "XY", offset = 4}
pc.sketch.circle{sketch = at, x = 5, y = 5, radius = 2}
local hole = pc.design.pocket{sketch = at, through_all = true}
-- Three holes 10 mm apart: 20 mm from the first to the last.
pc.design.linear_pattern{body = body, originals = {hole}, axis = "X", length = 20, occurrences = 3}
assert(#pc.doc.rebuild() == 0, "the pattern builds")
local xs = {}
for _, face in ipairs(pc.doc.faces{body = body}) do
  if face.kind == "cylinder" then xs[#xs + 1] = face.axis.point[1] end
end
table.sort(xs)
assert(#xs == 3 and math.abs(xs[1] - 5) < 1e-6 and math.abs(xs[3] - 25) < 1e-6)
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (30 * 10 - 3 * math.pi * 4) * 4) < 1e-3, volume)
```

`pc.design.polar_pattern`: Repeat the last feature about an axis.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- `originals = {id, ...}` are the features it repeats; left out, it takes the body's last feature that is not a dress-up, a pattern, a mirror or a boolean. It is refused in a body with no solid feature yet.
- A copy that lands outside the solid (a pocket repeated past the material) changes nothing, without an error.
- `axis` is Z through the body's origin when left out; `occurrences` (4) counts the original. `angle_deg` of 360 (the default) spreads them evenly round the turn; less puts the first and the last at its two ends.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.design.linear_pattern`.

Example: Six holes round a disc.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 20}
local disc = pc.design.pad{sketch = s, length = 3}
local body = pc.doc.feature{id = disc}.body
local at = pc.sketch.new{body = body, plane = "XY", offset = 3}
pc.sketch.circle{sketch = at, x = 12, y = 0, radius = 2}
local hole = pc.design.pocket{sketch = at, through_all = true}
-- Six holes around Z: 360 degrees is the whole turn, not a seventh copy on the first.
pc.design.polar_pattern{body = body, originals = {hole}, axis = "Z", angle_deg = 360, occurrences = 6}
assert(#pc.doc.rebuild() == 0, "the pattern builds")
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (math.pi * 400 - 6 * math.pi * 4) * 3) < 1e-3, volume)
```

`pc.design.scaled`: Scale the last feature.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- `originals = {id, ...}` are the features it repeats; left out, it takes the body's last feature that is not a dress-up, a pattern, a mirror or a boolean. It is refused in a body with no solid feature yet.
- A copy that lands outside the solid (a pocket repeated past the material) changes nothing, without an error.
- It makes a Multi Transform whose fields are `originals` and `steps`, one `{Scale = {factor, center, occurrences}}`: 1.5 about the body's origin, 2 occurrences, when left out. The occurrences count the original and grow evenly up to `factor`. A `factor` given on its own is refused.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.design.linear_pattern`.

Example: A block repeated at 1.5 and 2 times its size.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 10, y = 0, width = 10, height = 10}
local pad = pc.design.pad{sketch = s, length = 10}
local body = pc.doc.feature{id = pad}.body
-- Three occurrences, the original counted: scaled 1, 1.5 and 2 about the origin.
pc.design.scaled{body = body, originals = {pad},
  steps = {{Scale = {factor = 2, center = {0, 0, 0}, occurrences = 3}}}}
assert(#pc.doc.rebuild() == 0, "the scaled copies build")
local m = pc.doc.measure{body = body}
assert(math.abs(m.max[1] - 40) < 1e-6, m.max[1])
-- 1000 + 3375 + 8000, less the two overlaps (500 and 2250).
assert(math.abs(m.volume - 9625) < 1e-3, m.volume)
```

`pc.design.boolean`: Combine with another body.

- `sketch` (id, optional): The sketch it uses
- `body` (id, optional): The body it goes in; the sketch's body when left out
- `name` (string, optional): Its name in the tree
- `face_point` (list, optional): A face it takes as the viewport's picked face (a thickness's opening, a draft's neutral plane, a mirror's plane, the profile of a pad or a pocket given no sketch): a point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- Other arguments: Any field of the feature, such as length = 20 or reversed = true
- Returns the feature's id

Notes:

- It makes the feature and builds nothing: a feature that cannot build is told by `pc.doc.rebuild()`, in the list it returns, and its body stays the solid before it. A misspelt field is refused here, naming the fields the feature has.
- `body` is the one it changes; `tool_body` (the latest other body when left out) is what it takes, and keeps its own solid. It is refused while the document has no other body.
- `kind` is Fuse when left out, or Cut or Common; `more_tools = {id, ...}` takes further bodies the same way.

See also `pc.doc.rebuild`, `pc.design.set`, `pc.doc.new_body`, `pc.doc.set_visible`.

Example: A corner cut out of a block by another body.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 20}
local block = pc.design.pad{sketch = s, length = 10}
local body = pc.doc.feature{id = block}.body
local t = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = t, x = 20, y = 20, radius = 5}
local post = pc.design.pad{sketch = t, length = 30}
local tool = pc.doc.feature{id = post}.body
pc.design.boolean{body = body, tool_body = tool, kind = "Cut"}
assert(#pc.doc.rebuild() == 0, "the boolean builds")
-- A quarter of the post stands in the block's corner.
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (4000 - math.pi * 25 / 4 * 10)) < 1e-3, volume)
assert(pc.doc.measure{body = tool}.volume > 0, "the tool body keeps its own solid")
```

`pc.design.set`: Change fields of a Design feature or a datum.

- `feature` (id): The feature to change
- Other arguments: The fields to change, such as length = 25; a datum takes offset {x, y, z}, rotation and flip as design.datum does

Notes:

- It builds nothing: `pc.doc.rebuild()` builds the change and tells what fails. A misspelt field is refused, naming the fields the feature has.
- A sketch is not a Design feature and is refused; the `sketch.*` commands change it.

See also `pc.doc.feature`, `pc.doc.set_formula`.

Example: A pad made taller.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
pc.design.set{feature = pad, length = 8}
assert(pc.doc.feature{id = pad}.fields.Pad.length == 8)
assert(#pc.doc.rebuild() == 0, "the pad builds")
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - 20 * 10 * 8) < 1e-3, volume)
```

`pc.design.datum`: Add a datum plane, line, point or coordinate system.

- `kind` (string): plane, line, point or coordinate_system
- `body` (id): The body it belongs to
- `mode` (string, optional): What it attaches to: base_plane (the default), face, three_points, normal_to_edge, along_edge, two_points, plane_intersection, curve_centre or inertia; references are in the body's own frame and follow its solid
- `plane` (string, optional): base_plane: the base plane it sits on, XY (the default), XZ or YZ
- `face_point` (list, optional): face: a point of the face, {x, y, z}; without a mode, a flat face kept as given
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- `edge_point` (list, optional): normal_to_edge, along_edge, curve_centre: a point of the edge, {x, y, z}
- `edge_direction` (list, optional): With edge_point: the way the edge runs there, {x, y, z}
- `spot` (string, optional): normal_to_edge: where on the edge, picked (the default), start, end, middle or centre
- `points` (list, optional): three_points, two_points: each {x, y, z}, or {face_point, face_normal}, or {edge_point, edge_direction, spot}
- `planes` (list, optional): plane_intersection: two of XY, XZ, YZ, a datum's id, or {face_point, face_normal}
- `offset` (list, optional): Moved along its own x, y and normal, {x, y, z} in mm
- `rotation` (number, optional): Turned about its normal, degrees
- `flip` (boolean, optional): Turned to face the other way
- `tilt` (list, optional): Tilted about its own x-axis, then its y-axis, {x, y} in degrees
- `size` (number, optional): How large it draws, mm
- `name` (string, optional): Its name in the tree
- Returns the datum's id

Notes:

- A plane on a face (`mode = "face"`) has its origin at `face_point` and follows the face when the solid changes; a sketch stands on the datum with `pc.sketch.new{body = ..., on = datum}`.
- `design.set{feature = datum, offset = {x, y, z}}` moves it later, and what stands on it follows at the next `pc.doc.rebuild()`.

See also `pc.sketch.new`, `pc.design.set`.

Example: A boss on a plane that follows the top face.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
-- A plane on the top face, its origin where the face was given.
local plane = pc.design.datum{kind = "plane", body = body, mode = "face",
  face_point = {10, 5, 5}, face_normal = {0, 0, 1}}
local on = pc.sketch.new{body = body, on = plane}
pc.sketch.circle{sketch = on, x = 0, y = 0, radius = 2}
pc.design.pad{sketch = on, length = 3}
assert(#pc.doc.rebuild() == 0, "the boss builds")
assert(math.abs(pc.doc.measure{body = body}.volume - (1000 + math.pi * 4 * 3)) < 1e-3)
-- The plane follows the face: a taller block lifts the boss with it.
pc.design.set{feature = pad, length = 8}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.max[3] - 11) < 1e-6)
```

`pc.design.borrow`: Borrow another body's sketch, or faces and edges of its solid.

- `body` (id): The body that borrows
- `sketch` (id, optional): A sketch of another body: its profile, for this body's features
- `from` (id, optional): Or the body whose solid lends faces and edges
- `faces` (list, optional): With from: faces it lends, each {point, normal} in that body's own frame
- `edges` (list, optional): With from: edges it lends, each {point, direction} in that body's own frame
- `frozen` (boolean, optional): Keep the geometry as it is now rather than follow the source
- `options` (any, optional): How it lends: {offset = {translation = {x, y, z}, rotation_deg, tilt = {x, y}, flip}} moves and turns it along and about this body's axes; fill = true makes closed borrowed edges a face features take as a profile; whole = true lends the whole solid's edges as reference
- `name` (string, optional): Its name in the tree
- Returns the borrow's id

Notes:

- A feature takes a borrowed sketch by the borrow's id as its `sketch`. A pad or a pocket takes a lent face with `profile_borrowed = {borrow = id, index = 0}`, once the lending body is built (`pc.doc.rebuild()`); before, it is refused asking for a sketch.
- Lent faces serve elsewhere too: an up-to-shape pad or pocket stops on them with `extras = {up_to_shape_borrowed = {{borrow = id, index = 0}}}`, a revolution or groove turns until one with `mode = {UpToBorrowed = {borrow = id, index = 0}}`, and `design.map_sketch` puts a sketch on one.
- Borrowing a sketch of the same body is refused (a feature takes it directly), as is giving both `sketch` and `from`.

See also `pc.design.freeze`, `pc.design.map_sketch`, `pc.doc.faces`.

Example: A sketch and a face of one body padded in another.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local base = pc.design.pad{sketch = s, length = 5}
local lender = pc.doc.feature{id = base}.body
local body = pc.doc.new_body{}
-- The other body's sketch, borrowed: a feature takes the borrow as its sketch.
local lent = pc.design.borrow{body = body, sketch = s}
local copy = pc.design.pad{sketch = lent, length = 3}
assert(pc.doc.feature{id = copy}.body == body)
-- Its top face, borrowed once the lender is built: a pad takes it as its profile.
assert(#pc.doc.rebuild() == 0)
local face = pc.design.borrow{body = body, from = lender,
  faces = {{point = {10, 5, 5}, normal = {0, 0, 1}}}}
pc.design.pad{body = body, profile_borrowed = {borrow = face, index = 0}, length = 2}
assert(#pc.doc.rebuild() == 0, "both pads build")
local m = pc.doc.measure{body = body}
assert(math.abs(m.volume - 20 * 10 * (3 + 2)) < 1e-3, m.volume)
assert(math.abs(m.max[3] - 7) < 1e-6, "the second pad stands on the lent face")
```

`pc.design.recognize_holes`: Make the round holes of a body's solid Hole features: their faces deleted, and each set of alike holes drilled again from a sketch of their centres.

- `body` (id): The body
- Returns {holes, left, features}: the holes made features, the bores left as they are (counterbores, slots) and the features added

Notes:

- The body must be built first (`pc.doc.rebuild()`); a body with no round holes gets nothing, `holes = 0`.
- Per set of alike holes it adds one Delete Faces, a sketch of their centres and a Hole, the Hole last; `design.set` on that Hole resizes them all.

See also `pc.design.hole`, `pc.design.delete_faces`.

Example: Two bores made a Hole feature and widened.

```lua
local body = pc.doc.new_body{}
pc.design.primitive{body = body, kind = {Box = {length = 20, width = 10, height = 5}}}
local at = pc.sketch.new{body = body, plane = "XY", offset = 5}
pc.sketch.circle{sketch = at, x = 5, y = 5, radius = 2}
pc.sketch.circle{sketch = at, x = 15, y = 5, radius = 2}
pc.design.pocket{sketch = at, through_all = true}
assert(#pc.doc.rebuild() == 0)
local made = pc.design.recognize_holes{body = body}
-- The two alike bores: one Delete Faces, one sketch of centres and one Hole.
assert(made.holes == 2 and made.left == 0 and #made.features == 3)
assert(#pc.doc.rebuild() == 0, "the holes drill again")
local hole = made.features[3]
assert(pc.doc.feature{id = hole}.fields.Hole.diameter == 4)
pc.design.set{feature = hole, diameter = 6}
assert(#pc.doc.rebuild() == 0)
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (1000 - 2 * math.pi * 9 * 5)) < 1e-3, volume)
```

`pc.design.freeze`: Freeze borrowed geometry as it is now, or let it follow its source again.

- `feature` (id): The borrow
- `frozen` (boolean, optional): true (the default) takes the source as it is now; false follows it again

Notes:

- Any feature other than a borrow is refused.

See also `pc.design.borrow`.

Example: A borrowed sketch frozen, then followed again.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local body = pc.doc.new_body{}
local lent = pc.design.borrow{body = body, sketch = s}
pc.design.pad{sketch = lent, length = 3}
pc.design.freeze{feature = lent}
-- A rectangle added to the source after the freeze is not taken.
pc.sketch.rect{sketch = s, x = 30, y = 0, width = 10, height = 10}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.volume - 600) < 1e-3)
pc.design.freeze{feature = lent, frozen = false}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.volume - 900) < 1e-3, "following it again")
```

`pc.design.map_sketch`: Put an existing sketch on a flat face its body borrows from another body; the sketch then follows the face.

- `sketch` (id): The sketch
- `borrow` (id): A borrow of faces in the sketch's body
- `index` (integer, optional): Which of the borrow's faces, from 0; its first flat one when left out

Notes:

- The sketch keeps its geometry in its own coordinates and moves with the plane; a datum, mode or face it stood on before is left.
- A borrow of another body, a curved face, or a face the lender has not built yet (`pc.doc.rebuild()` first) is refused.

See also `pc.design.borrow`, `pc.sketch.set_plane`.

Example: A post's sketch moved onto a face another body lends.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 20}
local block = pc.design.pad{sketch = s, length = 10}
local lender = pc.doc.feature{id = block}.body
assert(#pc.doc.rebuild() == 0)
local body = pc.doc.new_body{}
local top = pc.design.borrow{body = body, from = lender,
  faces = {{point = {10, 10, 10}, normal = {0, 0, 1}}}}
local post = pc.sketch.new{body = body, plane = "XY"}
pc.sketch.circle{sketch = post, x = 10, y = 10, radius = 3}
pc.design.pad{sketch = post, length = 5}
pc.design.map_sketch{sketch = post, borrow = top}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.min[3] - 10) < 1e-6, "on the lent top")
-- The lender grows: the post rises with its top.
pc.design.set{feature = block, length = 15}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.min[3] - 15) < 1e-6, "follows the top")
```

`pc.design.move_to_body`: Move a feature into another body's history, with the sketch and datums only it uses.

- `feature` (id): The feature
- `body` (id): The body it goes to, in at its tip
- Returns the ids of the features moved, the given one last

Notes:

- It is refused when the feature reads a sketch other features read too; `design.duplicate` with `body` copies it there with a sketch of its own.

See also `pc.design.duplicate`, `pc.doc.new_body`.

Example: A boss moved into a body of its own.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
local top = pc.sketch.new{body = body, plane = "XY", offset = 5}
pc.sketch.rect{sketch = top, x = 0, y = 0, width = 5, height = 5}
local boss = pc.design.pad{sketch = top, length = 5}
local other = pc.doc.new_body{}
local moved = pc.design.move_to_body{feature = boss, body = other}
-- The boss went with the sketch only it used.
assert(#moved == 2 and moved[1] == top and moved[2] == boss)
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.volume - 1000) < 1e-3)
assert(math.abs(pc.doc.measure{body = other}.volume - 125) < 1e-3)
```

`pc.design.duplicate`: Make a copy of a feature, with its own copies of the sketches and datums it reads.

- `feature` (id): The feature
- `body` (id, optional): The body the copy goes in, at its tip (the feature's own when left out)
- Returns the ids of the features made, the copy of the given one last

Notes:

- The copy is made exactly where the feature is, so in the same body it adds nothing to the solid until `design.set` changes it.

See also `pc.design.set`, `pc.design.move_to_body`.

Example: A pad copied and turned to grow the other way.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
local made = pc.design.duplicate{feature = pad}
-- A sketch of its own and the copied pad, last.
assert(#made == 2 and made[1] ~= s)
local copy = made[2]
pc.design.set{feature = copy, reversed = true}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
assert(math.abs(m.volume - 1000) < 1e-3 and math.abs(m.min[3] + 5) < 1e-6, m.volume)
```

`pc.design.centre_line`: Measure the centre line of a tube-like solid between two of its faces.

- `body` (id): The body whose solid it runs through
- `from_point` (list): A point of the face it starts at, {x, y, z}, in the body's own frame
- `from_normal` (list): That face's outward normal, {x, y, z}
- `to_point` (list): A point of the face it ends at, {x, y, z}
- `to_normal` (list): That face's outward normal, {x, y, z}
- `tolerance` (number, optional): How closely it follows the sections' centres, mm (0.02 when left out)
- Returns {length, points, deviation, straight}: its length in mm, points along it in the body's frame, the largest distance measured from a section's centre to it, and whether it is one straight segment

Notes:

- The body must be built first (`pc.doc.rebuild()`). The two faces are its ends, each given by a point on it and its outward normal.

See also `pc.doc.faces`, `pc.doc.measure`.

Example: The centre line of a quarter bend.

```lua
local s = pc.sketch.new{plane = "XZ"}
pc.sketch.circle{sketch = s, x = 10, y = 0, radius = 2}
local bend = pc.design.revolve{sketch = s, angle_deg = 90}
assert(#pc.doc.rebuild() == 0, "the bend builds")
local body = pc.doc.feature{id = bend}.body
local line = pc.design.centre_line{body = body,
  from_point = {10, 0, 0}, from_normal = {0, -1, 0},
  to_point = {0, 10, 0}, to_normal = {-1, 0, 0}}
-- A quarter circle of radius 10.
assert(math.abs(line.length - math.pi * 10 / 2) < 0.05, line.length)
assert(not line.straight)
```

`pc.design.gear`: Make an involute spur gear's profile, outer or internal (ring): a sketch to pad.

- `body` (id, optional): The body it goes in; the selected one, else a new one
- `plane` (string, optional): The base plane it lies on: XY, XZ or YZ (a gear and a sprocket take XY, a shaft XZ)
- `face_point` (list, optional): Or a face it lies on, centred at this point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- `name` (string, optional): Its name in the tree
- Other arguments: module, teeth, pressure_angle_deg, profile_shift, addendum and dedendum (in modules), backlash, root_fillet (in modules), undercut (the root a rack cutter leaves; false for a fillet arc), bore, keyway = {on, width, depth} (0 for the standard key's by the bore), internal (true for a ring), rim (a ring's outside diameter)
- Returns the sketch's id

Notes:

- It makes only the sketch, whose curves its numbers fix: a pad or a revolution built from it makes the solid, and `sketch.generator` changes the numbers afterwards.
- Without `body` it goes in the selected body, else in a new one: in a script each call without `body` starts a body of its own.
- A field the generator lacks is refused, naming the ones it has; fields left out keep their defaults, which `pc.doc.feature{id = ...}.fields.generator` shows.
- `module` is in mm (2 when left out) and `teeth` 20: the pitch diameter is module times teeth, the tip diameter two modules more. It lies on XY centred on the origin, with a 5 mm `bore` (0 for none).
- A bore that does not fit inside the root circle is refused, and no body is made for it.
- A gear of few teeth (below about 17 at 20°) is undercut at its root as a rack cutter leaves it; `pc.sketch.generator` answers `undercut = true` for it. `keyway = {on = true}` cuts the standard key's keyway into the bore.

See also `pc.sketch.generator`, `pc.design.pad`.

Example: A 12-tooth gear padded 5 mm.

```lua
local gear = pc.design.gear{module = 2, teeth = 12, bore = 6}
assert(pc.sketch.generator{sketch = gear}.undercut, "12 teeth are undercut")
local pad = pc.design.pad{sketch = gear, length = 5}
assert(#pc.doc.rebuild() == 0, "the gear builds")
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
-- The tip circle is 2 * (12 + 2) = 28 mm across; a tooth points along X.
assert(math.abs(m.max[1] - 14) < 1e-3 and math.abs(m.min[1] + 14) < 1e-3)
assert(math.abs(m.max[3] - 5) < 1e-6)
```

`pc.design.sprocket`: Make a roller chain sprocket's profile (ISO 606 teeth): a sketch to pad.

- `body` (id, optional): The body it goes in; the selected one, else a new one
- `plane` (string, optional): The base plane it lies on: XY, XZ or YZ (a gear and a sprocket take XY, a shaft XZ)
- `face_point` (list, optional): Or a face it lies on, centred at this point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- `name` (string, optional): Its name in the tree
- Other arguments: pitch, roller (the roller's diameter), teeth, bore, keyway = {on, width, depth} (0 for the standard key's by the bore)
- Returns the sketch's id

Notes:

- It makes only the sketch, whose curves its numbers fix: a pad or a revolution built from it makes the solid, and `sketch.generator` changes the numbers afterwards.
- Without `body` it goes in the selected body, else in a new one: in a script each call without `body` starts a body of its own.
- A field the generator lacks is refused, naming the ones it has; fields left out keep their defaults, which `pc.doc.feature{id = ...}.fields.generator` shows.
- `pitch` and `roller` are the chain's, in mm (12.7 and 8.51 when left out), with 18 `teeth` and an 8 mm `bore`. It lies on XY centred on the origin.

See also `pc.sketch.generator`, `pc.design.pad`.

Example: A 9-tooth sprocket, its diameters read back.

```lua
local sprocket = pc.design.sprocket{teeth = 9, bore = 6}
local size = pc.sketch.generator{sketch = sprocket}
-- A chain's pitch circle: the pitch over the sine of half a tooth's angle.
assert(math.abs(size.pitch_diameter - 12.7 / math.sin(math.pi / 9)) < 1e-3)
local pad = pc.design.pad{sketch = sprocket, length = 3}
assert(#pc.doc.rebuild() == 0, "the sprocket builds")
```

`pc.design.shaft`: Make a stepped shaft's half section: a sketch to revolve about its vertical axis.

- `body` (id, optional): The body it goes in; the selected one, else a new one
- `plane` (string, optional): The base plane it lies on: XY, XZ or YZ (a gear and a sprocket take XY, a shaft XZ)
- `face_point` (list, optional): Or a face it lies on, centred at this point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- `name` (string, optional): Its name in the tree
- Other arguments: sections = {{length, diameter, chamfer, fillet}, ...}, start_chamfer, and loads = {bearings = {a, b}, forces = {{at, force, angle_deg}, ...}, torque (N·m), torque_from, torque_to, modulus (GPa)} for its stresses and deflection
- Returns the sketch's id

Notes:

- It makes only the sketch, whose curves its numbers fix: a pad or a revolution built from it makes the solid, and `sketch.generator` changes the numbers afterwards.
- Without `body` it goes in the selected body, else in a new one: in a script each call without `body` starts a body of its own.
- A field the generator lacks is refused, naming the ones it has; fields left out keep their defaults, which `pc.doc.feature{id = ...}.fields.generator` shows.
- It draws the half section on XZ, its sections from z = 0 upward, each a `length` and a `diameter` in mm, with a `chamfer` and a `fillet`; `start_chamfer` (0.5 mm when left out) breaks the bottom edge. `design.revolve` with no axis turns it about Z into the shaft.

See also `pc.sketch.generator`, `pc.design.revolve`.

Example: A two-step shaft turned from its section.

```lua
local section = pc.design.shaft{start_chamfer = 0,
  sections = {{length = 20, diameter = 10}, {length = 30, diameter = 16}}}
local turn = pc.design.revolve{sketch = section}
assert(#pc.doc.rebuild() == 0, "the shaft builds")
local m = pc.doc.measure{body = pc.doc.feature{id = turn}.body}
local volume = math.pi * (5 ^ 2 * 20 + 8 ^ 2 * 30)
assert(math.abs(m.volume - volume) < 1e-3, m.volume)
assert(math.abs(m.max[3] - 50) < 1e-6, "50 mm long, up Z")
```

`pc.design.keyway`: Make the slot of a parallel key in a shaft (DIN 6885): a sketch to pocket.

- `along` (list, optional): With face_point: the way it runs on the face, {x, y, z}, such as the shaft's axis
- `body` (id, optional): The body it goes in; the selected one, else a new one
- `plane` (string, optional): The base plane it lies on: XY, XZ or YZ (a gear and a sprocket take XY, a shaft XZ)
- `face_point` (list, optional): Or a face it lies on, centred at this point of it, {x, y, z}, in the body's own frame
- `face_normal` (list, optional): With face_point: the face's outward normal, {x, y, z}
- `name` (string, optional): Its name in the tree
- Other arguments: shaft (its diameter, which sizes the standard key), length, width and depth (0 for the standard key's), rounded (false for square ends)
- Returns the sketch's id

Notes:

- It makes only the sketch, whose curves its numbers fix: a pad or a revolution built from it makes the solid, and `sketch.generator` changes the numbers afterwards.
- Without `body` it goes in the selected body, else in a new one: in a script each call without `body` starts a body of its own.
- A field the generator lacks is refused, naming the ones it has; fields left out keep their defaults, which `pc.doc.feature{id = ...}.fields.generator` shows.
- The slot is `length` long end to end and as wide as the key, centred on the plane's origin or on `face_point`, running along x or `along`. `pc.sketch.generator{sketch = id}` answers its `width` and the `depth` to pocket it; a shaft outside 6 to 230 mm has no standard key and needs both given.
- On a shaft turned about Z with `design.shaft`, give the face as a point on its round side and the outward normal there, and `along = {0, 0, 1}`.

See also `pc.sketch.generator`, `pc.design.pocket`, `pc.design.shaft`.

Example: A keyway pocketed into a 20 mm shaft.

```lua
local section = pc.design.shaft{start_chamfer = 0, sections = {{length = 60, diameter = 20}}}
local turn = pc.design.revolve{sketch = section}
local body = pc.doc.feature{id = turn}.body
assert(#pc.doc.rebuild() == 0)
local before = pc.doc.measure{body = body}.volume
local key = pc.design.keyway{body = body, shaft = 20, length = 25,
  face_point = {10, 0, 30}, face_normal = {1, 0, 0}, along = {0, 0, 1}}
local size = pc.sketch.generator{sketch = key}
-- A 20 mm shaft takes a 6 x 6 key, 3.5 mm into the shaft.
assert(size.width == 6 and size.depth == 3.5)
pc.design.pocket{sketch = key, depth = size.depth}
assert(#pc.doc.rebuild() == 0, "the keyway cuts")
local m = pc.doc.measure{body = body}
-- The slot's footprint, 142.3 mm², at most 3.5 mm deep, and
-- no less than 3.04 at its sides, where the shaft curves away.
local taken = before - m.volume
assert(taken > 142.27 * 3.03 and taken < 142.27 * 3.5, taken)
assert(math.abs(m.max[3] - 60) < 1e-6, "the shaft keeps its length")
```

### surface

`pc.surface.extrude`: Extrude curves into a surface.

- `body` (id, optional): The surface body it goes in, or a feature in it; else its first sketch's body when that holds only drawings and surfaces, else a new one
- `name` (string, optional): Its name in the tree
- `sketches` (list, optional): The sketches it is built from: every chain of each, open or closed
- `length` (number, optional): mm (10)
- `direction` (any, optional): SketchNormal (the default), X, Y, Z or {Custom = {x, y, z}}
- `symmetric` (boolean, optional): Half each way
- `reversed` (boolean, optional): The other way
- `curves` (list, optional): In place of `sketches`: each {Sketch = id}, {Edge = {point, direction}} for an edge of the body, or {BodyEdge = {body = id, edge = {point, direction}}} for an edge of another body, in its frame
- Returns The new feature's id

Notes:

- It goes in `body` (a body, or any feature in it); without one, in its first sketch's body when that holds only sketches, datums and surfaces, else in a new body named Surface. A body Design builds is refused: surfaces go in a body of their own, and `pc.sketch.new{body = id}` starts a sketch in one.
- An open sheet has an area and no volume: `pc.doc.measure` gives `volume` nil until the body is sewn closed or thickened.
- `length` is 10 mm when left out. `direction` is SketchNormal (square to the first sketch), X, Y, Z or {Custom = {x, y, z}}; "Custom" without its numbers is refused with the form it takes. `symmetric = true` runs half each way, `reversed = true` the other way.
- Built from edges (`curves`) it needs a direction of its own: SketchNormal fails at rebuild ("the direction follows a sketch's plane").
- An edge of another body (`BodyEdge`, the edge as `pc.doc.edges{body = id}` lists it) is taken where that body sits, and the step builds again when that body changes or moves.

See also `pc.sketch.new`, `pc.surface.check`.

Example: Two walls from an open outline.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.polyline{sketch = s, points = {{10, 0}, {0, 0}, {0, 10}}}
local walls = pc.surface.extrude{sketches = {s}, length = 5}
assert(#pc.doc.rebuild() == 0)
local body = pc.doc.feature{id = walls}.body
assert(#pc.doc.faces{body = body} == 2, "a face per line")
local m = pc.doc.measure{body = body}
assert(math.abs(m.area - 100) < 1e-6 and m.volume == nil, "a sheet, open")
assert(math.abs(m.max[3] - 5) < 1e-6)

-- A wall standing on the top edge of the first, in a body of its own.
local top
for _, e in ipairs(pc.doc.edges{body = body}) do
  if math.abs(e.point[3] - 5) < 1e-6 and math.abs(e.point[2]) < 1e-6 then top = e end
end
local on = pc.surface.extrude{direction = "Z", length = 2, curves = {
  {BodyEdge = {body = body, edge = {point = top.point, direction = top.direction}}}}}
assert(#pc.doc.rebuild() == 0)
local other = pc.doc.measure{body = pc.doc.feature{id = on}.body}
assert(math.abs(other.min[3] - 5) < 1e-6 and math.abs(other.max[3] - 7) < 1e-6)
```

`pc.surface.revolve`: Revolve curves about an axis into a surface.

- `body` (id, optional): The surface body it goes in, or a feature in it; else its first sketch's body when that holds only drawings and surfaces, else a new one
- `name` (string, optional): Its name in the tree
- `sketches` (list, optional): The sketches it is built from: every chain of each, open or closed
- `angle_deg` (number, optional): Degrees (360)
- `axis` (any, optional): SketchVertical (the default), SketchHorizontal, X, Y, Z or {Custom = {origin = {x, y, z}, direction = {x, y, z}}}
- `curves` (list, optional): In place of `sketches`: each {Sketch = id}, {Edge = {point, direction}} for an edge of the body, or {BodyEdge = {body = id, edge = {point, direction}}} for an edge of another body, in its frame
- Returns The new feature's id

Notes:

- It goes in `body` (a body, or any feature in it); without one, in its first sketch's body when that holds only sketches, datums and surfaces, else in a new body named Surface. A body Design builds is refused: surfaces go in a body of their own, and `pc.sketch.new{body = id}` starts a sketch in one.
- An open sheet has an area and no volume: `pc.doc.measure` gives `volume` nil until the body is sewn closed or thickened.
- `angle_deg` is 360 when left out. `axis` is SketchVertical (the first sketch's vertical axis through its origin), SketchHorizontal, X, Y, Z or {Custom = {origin = {x, y, z}, direction = {x, y, z}}}.
- A curve lying along the axis fails at rebuild ("the whole wire lies along the axis, so it revolves out nothing").

See also `pc.sketch.new`, `pc.surface.check`.

Example: A cylinder's side from a line.

```lua
local s = pc.sketch.new{plane = "XZ"}
pc.sketch.line{sketch = s, x1 = 5, y1 = 0, x2 = 5, y2 = 10}
local tube = pc.surface.revolve{sketches = {s}}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = tube}.body}
assert(math.abs(m.area - 2 * math.pi * 5 * 10) < 1e-3, m.area)
assert(math.abs(m.min[1] + 5) < 1e-6 and math.abs(m.max[3] - 10) < 1e-6)
```

`pc.surface.planar`: Fill closed flat loops with a planar surface.

- `body` (id, optional): The surface body it goes in, or a feature in it; else its first sketch's body when that holds only drawings and surfaces, else a new one
- `name` (string, optional): Its name in the tree
- `sketches` (list, optional): The sketches whose closed loops it fills
- `curves` (list, optional): In place of `sketches`: each {Sketch = id}, {Edge = {point, direction}} for an edge of the body, or {BodyEdge = {body = id, edge = {point, direction}}} for an edge of another body, in its frame
- Returns The new feature's id

Notes:

- It goes in `body` (a body, or any feature in it); without one, in its first sketch's body when that holds only sketches, datums and surfaces, else in a new body named Surface. A body Design builds is refused: surfaces go in a body of their own, and `pc.sketch.new{body = id}` starts a sketch in one.
- An open sheet has an area and no volume: `pc.doc.measure` gives `volume` nil until the body is sewn closed or thickened.
- It fills closed flat loops, a loop inside another a hole. An open chain fails at rebuild ("profile wire is not closed").

See also `pc.sketch.new`, `pc.surface.sew`.

Example: A plate with a hole.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
pc.sketch.circle{sketch = s, x = 10, y = 5, radius = 2}
local plate = pc.surface.planar{sketches = {s}}
assert(#pc.doc.rebuild() == 0)
local body = pc.doc.feature{id = plate}.body
assert(#pc.doc.faces{body = body} == 1)
local area = pc.doc.measure{body = body}.area
assert(math.abs(area - (200 - math.pi * 4)) < 1e-3, area)
```

`pc.surface.fill`: Fill the hole curves close with a surface.

- `body` (id, optional): The surface body it goes in, or a feature in it; else its first sketch's body when that holds only drawings and surfaces, else a new one
- `name` (string, optional): Its name in the tree
- `sketches` (list, optional): The sketches whose curves close the hole, end to end
- `continuity` (string, optional): G0 (the default), G1 or G2: how it meets the faces of edges in `boundary`
- `boundary` (list, optional): In place of `sketches`: each {Sketch = id}, {Edge = {point, direction}} for an edge of the body, or {BodyEdge = {body = id, edge = {point, direction}}} for an edge of another body, in its frame
- Returns The new feature's id

Notes:

- It goes in `body` (a body, or any feature in it); without one, in its first sketch's body when that holds only sketches, datums and surfaces, else in a new body named Surface. A body Design builds is refused: surfaces go in a body of their own, and `pc.sketch.new{body = id}` starts a sketch in one.
- An open sheet has an area and no volume: `pc.doc.measure` gives `volume` nil until the body is sewn closed or thickened.
- The curves must meet end to end and close; ones that do not fail at rebuild ("the boundary does not close"). The loop may rise out of a plane, which `surface.planar` cannot fill.
- `continuity` (G0, G1 or G2) is how it meets the face of each edge given in `boundary`; a sketch's curve is only touched.

See also `pc.sketch.new`, `pc.surface.planar`.

Example: A disc filling a circle.

```lua
local s = pc.sketch.new{plane = "XY", offset = 20}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 5}
local cap = pc.surface.fill{sketches = {s}}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = cap}.body}
assert(math.abs(m.area - math.pi * 25) < 1e-3, m.area)
assert(math.abs(m.min[3] - 20) < 1e-6 and math.abs(m.max[3] - 20) < 1e-6)
```

`pc.surface.ruled`: Span two curves with straight lines.

- `body` (id, optional): The surface body it goes in, or a feature in it; else its first sketch's body when that holds only drawings and surfaces, else a new one
- `name` (string, optional): Its name in the tree
- `sketches` (list, optional): Two sketches: the first curve, then the second
- `first` (any, optional): {Sketch = id}, {Edge = {point, direction}} for an edge of the body, or {BodyEdge = {body = id, edge = {point, direction}}} for an edge of another body, in its frame
- `second` (any, optional): {Sketch = id}, {Edge = {point, direction}} for an edge of the body, or {BodyEdge = {body = id, edge = {point, direction}}} for an edge of another body, in its frame
- Returns The new feature's id

Notes:

- It goes in `body` (a body, or any feature in it); without one, in its first sketch's body when that holds only sketches, datums and surfaces, else in a new body named Surface. A body Design builds is refused: surfaces go in a body of their own, and `pc.sketch.new{body = id}` starts a sketch in one.
- An open sheet has an area and no volume: `pc.doc.measure` gives `volume` nil until the body is sewn closed or thickened.
- Exactly two curves, the first sketch's and the second's; a third is refused. Their ends pair start to start, as each was drawn: draw both the same way round, or the sheet twists.

See also `pc.sketch.new`, `pc.surface.loft`.

Example: A slanted strip between two lines.

```lua
local a = pc.sketch.new{plane = "XY"}
pc.sketch.line{sketch = a, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
local b = pc.sketch.new{plane = "XY", offset = 5}
pc.sketch.line{sketch = b, x1 = 0, y1 = 3, x2 = 10, y2 = 3}
local strip = pc.surface.ruled{sketches = {a, b}}
assert(#pc.doc.rebuild() == 0)
local area = pc.doc.measure{body = pc.doc.feature{id = strip}.body}.area
assert(math.abs(area - 10 * math.sqrt(34)) < 1e-3, area)
```

`pc.surface.loft`: Loft a surface through sections in order.

- `body` (id, optional): The surface body it goes in, or a feature in it; else its first sketch's body when that holds only drawings and surfaces, else a new one
- `name` (string, optional): Its name in the tree
- `sketches` (list, optional): The sections, a sketch each, in order: two or more
- `closed` (boolean, optional): Run on from the last section back to the first
- `sections` (list, optional): In place of `sketches`: each {Sketch = id}
- `guides` (list, optional): Curves the surface follows between the sections, each {Sketch = id} or {Edge = {point, direction}}, crossing every section once
- Returns The new feature's id

Notes:

- It goes in `body` (a body, or any feature in it); without one, in its first sketch's body when that holds only sketches, datums and surfaces, else in a new body named Surface. A body Design builds is refused: surfaces go in a body of their own, and `pc.sketch.new{body = id}` starts a sketch in one.
- An open sheet has an area and no volume: `pc.doc.measure` gives `volume` nil until the body is sewn closed or thickened.
- Two sections or more, a sketch each, passed through in order; `closed = true` runs on from the last back to the first. Sections may differ: a square to a circle gives a face per side.
- `guides` shape the surface between the sections: each crosses every section once, all in the same order, and the sections' ends or middles alike. A guided loft is one face through sections of one curve each; it does not close ("a loft that follows guides does not close"), and a guide missing a section fails at rebuild.

See also `pc.sketch.new`, `pc.surface.ruled`.

Example: A bulge through three circles.

```lua
local function ring(z, r)
  local s = pc.sketch.new{plane = "XY", offset = z}
  pc.sketch.circle{sketch = s, x = 0, y = 0, radius = r}
  return s
end
local loft = pc.surface.loft{sketches = {ring(0, 5), ring(10, 8), ring(20, 5)}}
assert(#pc.doc.rebuild() == 0)
local body = pc.doc.feature{id = loft}.body
assert(#pc.doc.faces{body = body} == 1)
local m = pc.doc.measure{body = body}
assert(math.abs(m.max[1] - 8) < 1e-3 and math.abs(m.max[3] - 20) < 1e-6)

-- Two lines, bowed between them by two arched guides.
local function edge(z)
  local s = pc.sketch.new{body = body, plane = "XY", offset = z}
  pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
  return s
end
local function guide(x)
  -- Through (0, 0), (3, 5) and (0, 10) on YZ: y, then z.
  local s = pc.sketch.new{body = body, plane = "YZ", offset = x}
  local r, a = 17 / 3, math.deg(math.atan(5, 8 / 3))
  pc.sketch.arc{sketch = s, x = 3 - r, y = 5, radius = r, start = -a, ["end"] = a}
  return {Sketch = s}
end
local bowed = pc.surface.loft{body = body, sketches = {edge(0), edge(10)},
  guides = {guide(0), guide(10)}}
assert(#pc.doc.rebuild() == 0)
local faces = pc.doc.faces{body = body}
assert(#faces == 2, "the bulge and the bowed sheet")
assert(math.abs(pc.doc.measure{body = body}.max[2] - 8) < 1e-3)
```

`pc.surface.sweep`: Sweep a profile along a path into a surface.

- `body` (id, optional): The surface body it goes in, or a feature in it; else its first sketch's body when that holds only drawings and surfaces, else a new one
- `name` (string, optional): Its name in the tree
- `sketches` (list, optional): The profile's sketch, then the path's
- `profile` (list, optional): In place of `sketches`: the profile, each {Sketch = id}
- `path` (list, optional): In place of `sketches`: the path, each {Sketch = id}
- `second_rail` (list, optional): A second rail, each {Sketch = id}: the profile runs from the path to it and rides both
- Returns The new feature's id

Notes:

- It goes in `body` (a body, or any feature in it); without one, in its first sketch's body when that holds only sketches, datums and surfaces, else in a new body named Surface. A body Design builds is refused: surfaces go in a body of their own, and `pc.sketch.new{body = id}` starts a sketch in one.
- An open sheet has an area and no volume: `pc.doc.measure` gives `volume` nil until the body is sewn closed or thickened.
- `sketches` is the profile's sketch, then the path's. A closed path fails at rebuild ("a sweep surface along a closed spine is not built"), which is what a circle profile and its path given the other way round meet.
- With `second_rail` the path is the first rail: the profile, an open chain, runs from the path's start to the second rail's, and at every point along them is turned and scaled so its ends ride the rails. Ends off the rails' starts fail at rebuild ("the profile's ends must sit on the rails' starts").

See also `pc.sketch.new`, `pc.surface.loft`.

Example: A tube along a line.

```lua
local profile = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = profile, x = 0, y = 0, radius = 2}
local path = pc.sketch.new{plane = "XZ"}
pc.sketch.line{sketch = path, x1 = 0, y1 = 0, x2 = 0, y2 = 20}
local tube = pc.surface.sweep{sketches = {profile, path}}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = tube}.body}
assert(math.abs(m.area - 2 * math.pi * 2 * 20) < 1e-3, m.area)
assert(math.abs(m.max[3] - 20) < 1e-6)

-- An arch between two rails that spread from 10 apart to 14.
local arch = pc.sketch.new{plane = "XZ"}
pc.sketch.arc{sketch = arch, x = 5, y = 0, radius = 5, start = 0, ["end"] = 180}
local body = pc.doc.feature{id = arch}.body
local left = pc.sketch.new{body = body, plane = "XY"}
pc.sketch.line{sketch = left, x1 = 10, y1 = 0, x2 = 14, y2 = 20}
local right = pc.sketch.new{body = body, plane = "XY"}
pc.sketch.line{sketch = right, x1 = 0, y1 = 0, x2 = 0, y2 = 20}
pc.surface.sweep{body = body, sketches = {arch, left},
  second_rail = {{Sketch = right}}}
assert(#pc.doc.rebuild() == 0)
m = pc.doc.measure{body = body}
assert(math.abs(m.max[1] - 14) < 1e-2 and math.abs(m.max[3] - 7) < 2e-2,
  "wider and taller at the far end")
```

`pc.surface.offset`: Copy faces at a distance along their normals.

- `body` (id): The surface body it works on, or a feature in it
- `name` (string, optional): Its name in the tree
- `faces` (list, optional): The faces to copy, each {point, normal} as pc.doc.faces lists a face
- `distance` (number, optional): mm along the faces' normals (1); negative the other way
- Returns The new feature's id

Notes:

- `body` is required, a body or any feature in it: the step works on the surfaces the body holds before it.
- `faces` are faces of the body as `pc.doc.faces` lists them (the entry itself will do; a curved face has no `normal` and needs none). The copy is added beside them, the faces kept. `distance` is 1 mm when left out.

See also `pc.doc.faces`, `pc.surface.thicken`.

Example: A sheet copied 3 mm up.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
local body = pc.doc.feature{id = pc.surface.planar{sketches = {s}}}.body
assert(#pc.doc.rebuild() == 0)
local face = pc.doc.faces{body = body}[1]
pc.surface.offset{body = body, faces = {face}, distance = 3}
assert(#pc.doc.rebuild() == 0)
assert(#pc.doc.faces{body = body} == 2, "the sheet and its copy")
assert(math.abs(pc.doc.measure{body = body}.max[3] - 3) < 1e-6)
```

`pc.surface.extend`: Extend faces past picked edges.

- `body` (id): The surface body it works on, or a feature in it
- `name` (string, optional): Its name in the tree
- `edges` (list, optional): The free edges to grow past, each {point, direction}, as pc.doc.edges lists an edge
- `length` (number, optional): mm (5)
- `continuity` (string, optional): G1 (the default), G0 or G2
- Returns The new feature's id

Notes:

- `body` is required, a body or any feature in it: the step works on the surfaces the body holds before it.
- `edges` are free edges of the body's sheets, each {point = e.point, direction = e.direction} from `pc.doc.edges`; each face grows past them by `length` (5 mm) and stays one face.
- `continuity` G1 (the default) and G0 run straight on, G2 along the face's own surface. Past a cylinder's straight side, a curved direction, only G2 builds; G1 and G0 fail at rebuild ("a linear extension of an analytic surface across a curved parameter line would leave the surface").

See also `pc.doc.edges`.

Example: A sheet grown 5 mm past one side.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
local body = pc.doc.feature{id = pc.surface.planar{sketches = {s}}}.body
assert(#pc.doc.rebuild() == 0)
local side
for _, e in ipairs(pc.doc.edges{body = body}) do
  if math.abs(e.point[1] - 10) < 1e-6 then side = e end
end
pc.surface.extend{body = body, length = 5,
  edges = {{point = side.point, direction = side.direction}}}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
assert(math.abs(m.area - 150) < 1e-6 and math.abs(m.max[1] - 15) < 1e-6)
assert(#pc.doc.faces{body = body} == 1, "still one face")
```

`pc.surface.blend`: Bridge two edges with a surface.

- `body` (id): The surface body it works on, or a feature in it
- `name` (string, optional): Its name in the tree
- `first` (any, optional): {point, direction}, as pc.doc.edges lists an edge
- `second` (any, optional): {point, direction}, as pc.doc.edges lists an edge
- `continuity` (string, optional): G1 (the default), G0 or G2
- Returns The new feature's id

Notes:

- `body` is required, a body or any feature in it: the step works on the surfaces the body holds before it.
- `first` and `second` are edges of the body's sheets, each {point, direction} from `pc.doc.edges`; the blend is a new face between them, meeting each edge's face with `continuity` (G1 by default, G0 or G2).

See also `pc.doc.edges`, `pc.surface.fill`.

Example: A bridge between two strips.

```lua
local a = pc.sketch.new{plane = "XY", offset = 0}
pc.sketch.line{sketch = a, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
local low = pc.surface.extrude{sketches = {a}, length = 5, direction = "Y"}
local body = pc.doc.feature{id = low}.body
local b = pc.sketch.new{body = body, plane = "XY", offset = 10}
pc.sketch.line{sketch = b, x1 = 0, y1 = 10, x2 = 10, y2 = 10}
pc.surface.extrude{body = body, sketches = {b}, length = 5, direction = "Y"}
assert(#pc.doc.rebuild() == 0)
local first, second
for _, e in ipairs(pc.doc.edges{body = body}) do
  if math.abs(e.point[2] - 5) < 1e-6 and math.abs(e.point[3]) < 1e-6 then first = e end
  if math.abs(e.point[2] - 10) < 1e-6 and math.abs(e.point[3] - 10) < 1e-6 then second = e end
end
pc.surface.blend{body = body,
  first = {point = first.point, direction = first.direction},
  second = {point = second.point, direction = second.direction}}
assert(#pc.doc.rebuild() == 0)
assert(#pc.doc.faces{body = body} == 3, "two strips and the bridge")
assert(pc.doc.measure{body = body}.area > 200)
```

`pc.surface.split`: Split faces along curves.

- `body` (id): The surface body it works on, or a feature in it
- `name` (string, optional): Its name in the tree
- `sketches` (list, optional): The sketches whose curves cut the faces
- `faces` (list, optional): The faces to cut, each {point, normal} as pc.doc.faces lists a face
- `curves` (list, optional): In place of `sketches`: each {Sketch = id}
- Returns The new feature's id

Notes:

- `body` is required, a body or any feature in it: the step works on the surfaces the body holds before it.
- `faces` are faces of the body as `pc.doc.faces` lists them. A sketch's curve lands on the face as seen square to the sketch's plane, so a sketch above the face cuts it too.
- Each curve must cross the face from edge to edge; one ending inside fails at rebuild ("it must run from boundary to boundary").

See also `pc.doc.faces`, `pc.surface.trim`.

Example: A sheet cut in two along a line.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
local body = pc.doc.feature{id = pc.surface.planar{sketches = {s}}}.body
assert(#pc.doc.rebuild() == 0)
local face = pc.doc.faces{body = body}[1]
local cut = pc.sketch.new{body = body, plane = "XY", offset = 5}
pc.sketch.line{sketch = cut, x1 = 4, y1 = -1, x2 = 4, y2 = 11}
pc.surface.split{body = body, faces = {face}, sketches = {cut}}
assert(#pc.doc.rebuild() == 0)
local areas = {}
for _, f in ipairs(pc.doc.faces{body = body}) do areas[#areas + 1] = f.area end
table.sort(areas)
assert(#areas == 2 and math.abs(areas[1] - 40) < 1e-3 and math.abs(areas[2] - 60) < 1e-3)
```

`pc.surface.sew`: Sew the body's surfaces together.

- `body` (id): The surface body it works on, or a feature in it
- `name` (string, optional): Its name in the tree
- `gap` (number, optional): mm: edges this far apart are joined too (0, only edges that meet)
- Returns The new feature's id

Notes:

- `body` is required, a body or any feature in it: the step works on the surfaces the body holds before it.
- It joins every face of the body where their edges meet; `gap` (mm) joins edges up to that far apart too.
- A shell that closes becomes a solid, with a volume. One that does not stays a sheet, its faces now sharing edges: `surface.check` lists those joins and `surface.fillet` can round them.

See also `pc.surface.check`, `pc.surface.fillet`, `pc.surface.planar`.

Example: Two walls joined at their corner.

```lua
local a = pc.sketch.new{plane = "XY"}
pc.sketch.line{sketch = a, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
local wall = pc.surface.extrude{sketches = {a}, length = 5}
local body = pc.doc.feature{id = wall}.body
local b = pc.sketch.new{body = body, plane = "XY"}
pc.sketch.line{sketch = b, x1 = 0, y1 = 10, x2 = 0, y2 = 0}
pc.surface.extrude{body = body, sketches = {b}, length = 5}
assert(#pc.doc.rebuild() == 0)
assert(#pc.surface.check{body = body} == 0, "touching, not joined")
pc.surface.sew{body = body}
assert(#pc.doc.rebuild() == 0)
assert(#pc.surface.check{body = body} == 1, "one shared edge")
```

`pc.surface.fillet`: Round edges where two faces of a surface meet.

- `body` (id): The surface body it works on, or a feature in it
- `name` (string, optional): Its name in the tree
- `edges` (list, optional): The edges to round, each {point, direction}, as pc.doc.edges lists an edge
- `radius` (number, optional): mm (2)
- Returns The new feature's id

Notes:

- `body` is required, a body or any feature in it: the step works on the surfaces the body holds before it.
- `edges` are edges two faces of the body share, each {point, direction} from `pc.doc.edges`; `radius` is 2 mm when left out.
- Separate sheets that only touch share no edge: the round fails at rebuild ("Sew them first so they share it"). Sew them, or draw the faces from one sketch.

See also `pc.surface.sew`, `pc.doc.edges`.

Example: Two walls rounded at their corner.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.polyline{sketch = s, points = {{10, 0}, {0, 0}, {0, 10}}}
local walls = pc.surface.extrude{sketches = {s}, length = 5}
local body = pc.doc.feature{id = walls}.body
assert(#pc.doc.rebuild() == 0)
local corner
for _, e in ipairs(pc.doc.edges{body = body}) do
  if #e.faces == 2 then corner = e end
end
pc.surface.fillet{body = body, radius = 2,
  edges = {{point = corner.point, direction = corner.direction}}}
assert(#pc.doc.rebuild() == 0)
assert(#pc.doc.faces{body = body} == 3, "two walls and the round")
local area = pc.doc.measure{body = body}.area
assert(math.abs(area - (100 - 4 * 5 + math.pi * 5)) < 1e-3, area)
```

`pc.surface.fillet_faces`: Round between two faces that share no edge.

- `body` (id): The surface body it works on, or a feature in it
- `name` (string, optional): Its name in the tree
- `first` (any, optional): A face of the body, {point, normal} as pc.doc.faces lists a face
- `second` (any, optional): Another face of the body, the same way
- `radius` (number, optional): mm (2)
- `flip_first` (boolean, optional): Roll the round on the first face's other side
- `flip_second` (boolean, optional): Roll the round on the second face's other side
- Returns The new feature's id

Notes:

- `body` is required, a body or any feature in it: the step works on the surfaces the body holds before it.
- `first` and `second` are faces of the body as `pc.doc.faces` lists them, that need share no edge: two separate surfaces. Each is cut back to where the round touches it and the three are sewn into one sheet; `radius` is 2 mm when left out.
- The round rolls on the side each face's normal points to; `flip_first` or `flip_second` rolls it on that face's other side. A side the ball cannot reach both faces from fails at rebuild ("rounding between the faces failed").

See also `pc.surface.fillet`, `pc.doc.faces`.

Example: Two walls of an L that stop short of the corner, rounded together.

```lua
local a = pc.sketch.new{plane = "XY"}
pc.sketch.line{sketch = a, x1 = 2, y1 = 0, x2 = 10, y2 = 0}
local body = pc.doc.feature{id = pc.surface.extrude{sketches = {a}, length = 5}}.body
local b = pc.sketch.new{body = body, plane = "XY"}
pc.sketch.line{sketch = b, x1 = 0, y1 = 10, x2 = 0, y2 = 2}
pc.surface.extrude{body = body, sketches = {b}, length = 5}
assert(#pc.doc.rebuild() == 0)
local first, second
for _, f in ipairs(pc.doc.faces{body = body}) do
  if math.abs(f.point[2]) < 1e-6 then first = f else second = f end
end
pc.surface.fillet_faces{body = body, first = first, second = second, radius = 3,
  flip_first = true, flip_second = true}
assert(#pc.doc.rebuild() == 0)
assert(#pc.doc.faces{body = body} == 3, "two walls and the round")
assert(#pc.surface.check{body = body} == 2, "one sheet")
local area = pc.doc.measure{body = body}.area
assert(math.abs(area - (2 * 7 * 5 + math.pi * 3 / 2 * 5)) < 1e-2, area)
```

`pc.surface.thicken`: Thicken the body's surfaces into solids.

- `body` (id): The surface body it works on, or a feature in it
- `name` (string, optional): Its name in the tree
- `thickness` (number, optional): mm along the faces' normals (2); negative the other way
- `both_sides` (boolean, optional): Half each side
- Returns The new feature's id

Notes:

- `body` is required, a body or any feature in it: the step works on the surfaces the body holds before it.
- Each sheet becomes a solid: `thickness` (2 mm) along the faces' normals, negative the other way, or half each way with `both_sides = true`.
- A body already sewn into a solid is refused at rebuild ("the body has no sheet to thicken"): Design's Thickness hollows a solid.

See also `pc.surface.sew`, `pc.surface.offset`.

Example: A sheet given 2 mm.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
local body = pc.doc.feature{id = pc.surface.planar{sketches = {s}}}.body
pc.surface.thicken{body = body, thickness = 2}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
assert(math.abs(m.volume - 200) < 1e-6, m.volume)
assert(#pc.doc.faces{body = body} == 6, "a box")
```

`pc.surface.trim`: Keep what of the body lies on one side of a plane.

- `body` (id): The surface body it works on, or a feature in it
- `name` (string, optional): Its name in the tree
- `plane` (any, optional): YZ (the default), XZ, XY or {Custom = {origin = {x, y, z}, normal = {x, y, z}}}, in the body's frame
- `offset` (number, optional): mm along the plane's normal
- `flip` (boolean, optional): Keep the side the normal points away from
- Returns The new feature's id

Notes:

- `body` is required, a body or any feature in it: the step works on the surfaces the body holds before it.
- It keeps what lies on the side the plane's normal points to: YZ keeps +X. `flip = true` keeps the other side; `offset` moves the plane along its normal, YZ with 4 being x = 4.
- A plane that leaves nothing fails at rebuild ("the plane leaves nothing of the body on the side kept; flip it or move it"). It cuts a sewn solid too, which stays solid.

See also `pc.surface.split`, `pc.surface.mirror`.

Example: A sheet cut at x = 4, either side kept.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
local body = pc.doc.feature{id = pc.surface.planar{sketches = {s}}}.body
local trim = pc.surface.trim{body = body, plane = "YZ", offset = 4}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
assert(math.abs(m.area - 60) < 1e-6 and math.abs(m.min[1] - 4) < 1e-6)
pc.surface.set{feature = trim, flip = true}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.area - 40) < 1e-6)
```

`pc.surface.mirror`: Add the body's reflection in a plane.

- `body` (id): The surface body it works on, or a feature in it
- `name` (string, optional): Its name in the tree
- `plane` (any, optional): YZ (the default), XZ, XY or {Custom = {origin = {x, y, z}, normal = {x, y, z}}}, in the body's frame
- `offset` (number, optional): mm along the plane's normal
- Returns The new feature's id

Notes:

- `body` is required, a body or any feature in it: the step works on the surfaces the body holds before it.
- It adds the body's reflection beside what the body holds, the original kept; the two are separate pieces of the one body.
- `plane` YZ (the default), XZ, XY or {Custom = {origin, normal}}, in the body's frame; `offset` moves it along its normal: YZ with 10 reflects in x = 10.

See also `pc.surface.trim`.

Example: A sheet and its reflection.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
local body = pc.doc.feature{id = pc.surface.planar{sketches = {s}}}.body
pc.surface.mirror{body = body, plane = "YZ"}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
assert(math.abs(m.area - 200) < 1e-6 and math.abs(m.min[1] + 10) < 1e-6)
assert(#pc.doc.faces{body = body} == 2)
```

`pc.surface.check`: Measure how a body's faces meet at each shared edge.

- `body` (id): The body, or a feature in it
- Returns a list of {point, gap, angle_deg, curvature, join}, one per shared edge; the view labels them

Notes:

- One entry per edge two faces of the body share: `gap` in mm between them, `angle_deg` the crease (0 where they meet tangent), `point` halfway along the edge. Any body with a shape is checked, a Design solid too.
- `curvature` is the largest difference, in 1/mm, between how sharply the two faces bend square to the edge (nil where it could not be read); `join` is how the view labels it: G2 where they meet tangent and bend alike, G1 tangent, else the crease angle, or the gap where they are apart.
- Sheets that only touch share no edge until sewn, so the seam between them is not listed: two separate sheets give an empty list. A body not built yet is refused ("The body has no shape to check yet").

See also `pc.surface.sew`, `pc.surface.fillet`.

Example: A sharp corner, then rounded.

```lua
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
```

`pc.surface.curvature`: Paint a body with how sharply its surfaces bend.

- `body` (id): The body, or a feature in it
- `measure` (string, optional): gaussian (the default), mean, max or min
- `limit` (number, optional): The curvature the colours reach at either end; 0 (the default) takes it from the body
- Returns {measure, low, high, unit}: the measure's range over the body

Notes:

- `mean`, `max` and `min` are signed against the faces' outward normals: negative where a surface bulges out, positive in a hollow. `gaussian` is the product of the two principal curvatures (1/mm²): positive on a dome or in a bowl, negative on a saddle, zero on a plane and on what unrolls flat (a cylinder, a cone). `mean` is their average, `max` and `min` each (1/mm).
- The view paints the body until the task closes, or until `surface.zebra` or `surface.check` takes its place. A body not built yet is refused ("The body has no shape to paint yet").

See also `pc.surface.zebra`, `pc.surface.check`.

Example: A tube's side bends one way only.

```lua
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
```

`pc.surface.zebra`: Paint a body with zebra stripes.

- `body` (id): The body, or a feature in it
- `stripes` (number, optional): Dark stripes per half turn of the surface (6), 1 to 64
- `axis` (string, optional): X, Y or Z (the default): the way they run

Notes:

- Each stripe is a band of the way the surface faces, turned about `axis`. Across a crease the stripes break, across a tangent join they bend sharply, and across a curvature continuous one they run on smoothly. The view paints the body until the task closes.

See also `pc.surface.curvature`, `pc.surface.check`.

Example: Stripes on a tube.

```lua
local s = pc.sketch.new{plane = "XZ"}
pc.sketch.line{sketch = s, x1 = 5, y1 = 0, x2 = 5, y2 = 10}
local body = pc.doc.feature{id = pc.surface.revolve{sketches = {s}}}.body
assert(#pc.doc.rebuild() == 0)
assert(pc.surface.zebra{body = body, stripes = 8} == nil)
local ok = pcall(pc.surface.zebra, {body = body, axis = "W"})
assert(not ok, "X, Y or Z")
```

`pc.surface.set`: Change fields of a surface step.

- `feature` (id): The surface step to change
- Other arguments: The fields to change, by name (`length`, `continuity`, `plane`, `sketches`…), as the step's own command takes them

Notes:

- It takes the fields the step's command takes, by the same names; `sketches` replaces the curves it is built from. A name the step does not have is refused ("is not a field of this surface"); the kind of step stays.
- The step keeps its place in the body's history: everything after it builds again on the change at `pc.doc.rebuild()`.

Example: A wall made taller.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
local wall = pc.surface.extrude{sketches = {s}, length = 5}
local body = pc.doc.feature{id = wall}.body
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.area - 50) < 1e-6)
pc.surface.set{feature = wall, length = 8}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.area - 80) < 1e-6)
```

### asm

`pc.asm.mate`: Put two flat faces against each other.

- `body` (id): The body that moves
- `face` (any): A flat face, {point, normal}, as pc.doc.faces lists it
- `other` (id): The body it is held against; the nil id (all zeros) for the world origin, its faces then in world space
- `other_face` (any): A flat face, {point, normal}, as pc.doc.faces lists it
- `name` (string, optional): Its name in the tree
- `offset` (number, optional): The gap between them, mm
- `flip` (boolean, optional): Face the same way instead of at each other
- Returns the joint's id

Notes:

- `body` moves and `other` stays. The document's first joint also grounds `other` (as `asm.ground` does), unless it is the world: the nil id, all zeros, whose faces are given in world space.
- Faces are given where the bodies sit now, in world space, as `pc.doc.faces` lists them; the joint keeps them in each body's own frame and solves as it is made. A joint that cannot hold with the others is not made: the call fails naming the joints in conflict.
- `offset` is the gap between the faces in mm, 0 when left out; `flip = true` turns the body so both faces point the same way.
- It holds the faces together and nothing else: the body keeps a turn about the normal and two slides along the face, so it is not centred and stays where it was across the face.
- Both faces must be flat: a round face is refused as having no normal.

See also `pc.asm.set`, `pc.asm.distance`, `pc.asm.flip`.

Example: A lid set on a base.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local function facing(body, z)
  for _, f in ipairs(pc.doc.faces{body = body}) do
    if f.normal and f.normal[3] * z > 0.99 then return f end
  end
end
local base = box(0, 20, 20, 5)
local lid = box(40, 10, 10, 3)
assert(#pc.doc.rebuild() == 0)
pc.asm.mate{body = lid, face = facing(lid, -1),
  other = base, other_face = facing(base, 1)}
local at = pc.asm.placement{body = lid}.translation
assert(at[1] == 0 and at[2] == 0 and math.abs(at[3] - 5) < 1e-4,
  "lifted onto the top, not moved across")
assert(pc.asm.freedom{body = lid}[1].free == 3,
  "it may still slide and turn on the face")
```

`pc.asm.align`: Put two round faces on one axis.

- `body` (id): The body that moves
- `face` (any): A round face, {axis = {point, direction}}, as pc.doc.faces lists it; an edge's line or circle axis goes the same way
- `other` (id): The body it is held against; the nil id (all zeros) for the world origin, its faces then in world space
- `other_face` (any): A round face, {axis = {point, direction}}, as pc.doc.faces lists it; an edge's line or circle axis goes the same way
- `name` (string, optional): Its name in the tree
- `turn_drive` (any, optional): An alignment's turn (degrees from where it was made) to hold it at; false lets it turn
- `turn_limits` (any, optional): {low, high}: the range an alignment's turn stays in; false takes it away
- `slide_drive` (any, optional): How far along the axis (mm) to hold an alignment; false lets it slide
- `slide_limits` (any, optional): {low, high}: the range an alignment's slide stays in, mm; false takes it away
- Returns the joint's id

Notes:

- `body` moves and `other` stays. The document's first joint also grounds `other` (as `asm.ground` does), unless it is the world: the nil id, all zeros, whose faces are given in world space.
- Faces are given where the bodies sit now, in world space, as `pc.doc.faces` lists them; the joint keeps them in each body's own frame and solves as it is made. A joint that cannot hold with the others is not made: the call fails naming the joints in conflict.
- The two axes go on one line; the body keeps a turn about it and a slide along it.
- `turn_drive` (degrees from where it was made) and `slide_drive` (mm) hold those motions; `turn_limits` and `slide_limits` keep them in a range; false takes each away.
- `asm.travel` refuses an alignment: it reads a hinge or a slider, which have one motion.

See also `pc.asm.set`, `pc.asm.hinge`, `pc.asm.slider`.

Example: A wheel on a shaft, held 8 mm up.

```lua
local function pin(x, r, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.circle{sketch = s, x = x, y = 0, radius = r}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local function round(body)
  for _, f in ipairs(pc.doc.faces{body = body}) do
    if f.axis then return f end
  end
end
local shaft = pin(0, 5, 20)
local wheel = pin(40, 15, 4)
assert(#pc.doc.rebuild() == 0)
pc.asm.align{body = wheel, face = round(wheel),
  other = shaft, other_face = round(shaft), slide_drive = 8}
local at = pc.asm.placement{body = wheel}.translation
assert(math.abs(at[1] + 40) < 1e-3 and math.abs(at[3] - 8) < 1e-3,
  "on the shaft's axis, 8 mm up")
assert(pc.asm.freedom{body = wheel}[1].free == 1,
  "the slide is held, the turn is free")
```

`pc.asm.angle`: Hold two faces or axes at an angle.

- `body` (id): The body that moves
- `face` (any): A flat face {point, normal} or a round face or edge {axis = {point, direction}}
- `other` (id): The body it is held against; the nil id (all zeros) for the world origin, its faces then in world space
- `other_face` (any): A flat face {point, normal} or a round face or edge {axis = {point, direction}}
- `name` (string, optional): Its name in the tree
- `degrees` (number, optional): Between their outward normals; the angle they make now when left out
- Returns the joint's id

Notes:

- `body` moves and `other` stays. The document's first joint also grounds `other` (as `asm.ground` does), unless it is the world: the nil id, all zeros, whose faces are given in world space.
- Faces are given where the bodies sit now, in world space, as `pc.doc.faces` lists them; the joint keeps them in each body's own frame and solves as it is made. A joint that cannot hold with the others is not made: the call fails naming the joints in conflict.
- `degrees` is between the outward normals, or the axes; left out, the angle they make now is kept.
- It holds only the angle: the body may still slide every way and turn about the other axes, five motions left.

See also `pc.asm.set`, `pc.asm.parallel`, `pc.asm.perpendicular`.

Example: A plate held at 30 degrees to a base.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local function facing(body, z)
  for _, f in ipairs(pc.doc.faces{body = body}) do
    if f.normal and f.normal[3] * z > 0.99 then return f end
  end
end
local base = box(0, 20, 20, 5)
local plate = box(40, 10, 10, 2)
assert(#pc.doc.rebuild() == 0)
pc.asm.angle{body = plate, face = facing(plate, 1),
  other = base, other_face = facing(base, 1), degrees = 30}
local w = pc.asm.placement{body = plate}.rotation[4]
assert(math.abs(math.deg(2 * math.acos(w)) - 30) < 1e-3, "turned 30 degrees")
assert(pc.asm.freedom{body = plate}[1].free == 5, "only the angle is held")
```

`pc.asm.hinge`: Put two axes on one line: the body can only turn about it.

- `body` (id): The body that moves
- `face` (any): A round face, {axis = {point, direction}}, as pc.doc.faces lists it; an edge's line or circle axis goes the same way
- `other` (id): The body it is held against; the nil id (all zeros) for the world origin, its faces then in world space
- `other_face` (any): A round face, {axis = {point, direction}}, as pc.doc.faces lists it; an edge's line or circle axis goes the same way
- `name` (string, optional): Its name in the tree
- `offset` (number, optional): How far along the axis the first sits from the second, mm
- `drive` (any, optional): A hinge's angle (degrees from where it was made) or a slider's position (mm) to hold it at; false lets it move again
- `limits` (any, optional): {low, high}: the range a hinge's angle or a slider's position stays in while not driven; false takes the limits away
- Returns the joint's id

Notes:

- `body` moves and `other` stays. The document's first joint also grounds `other` (as `asm.ground` does), unless it is the world: the nil id, all zeros, whose faces are given in world space.
- Faces are given where the bodies sit now, in world space, as `pc.doc.faces` lists them; the joint keeps them in each body's own frame and solves as it is made. A joint that cannot hold with the others is not made: the call fails naming the joints in conflict.
- The axes go on one line and the body keeps one motion, the turn about it. `offset` is how far along the axis the first sits from the second, mm.
- `drive` holds the angle in degrees from where the hinge was made, positive turning right-handed about the axis's direction; false lets it turn again.
- `limits = {low, high}` keeps the angle in that range while it is not driven; a body outside it is brought to the nearer end.

See also `pc.asm.set`, `pc.asm.travel`, `pc.asm.turn`, `pc.asm.couple`, `pc.asm.motion`.

Example: An arm on a post, turned a quarter turn.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
local a = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = a, x = -2, y = 20, width = 30, height = 4}
local arm = pc.doc.feature{id = pc.design.pad{sketch = a, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
local axis = {axis = {point = {0, 22, 0}, direction = {0, 0, 1}}}
local post_axis = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
local h = pc.asm.hinge{body = arm, face = axis,
  other = post, other_face = post_axis}
assert(pc.asm.freedom{body = arm}[1].free == 1, "it turns about the post")
pc.asm.set{joint = h, drive = 90}
assert(math.abs(pc.asm.travel{joint = h} - 90) < 1e-3)
local q = pc.asm.placement{body = arm}.rotation
assert(math.abs(q[3] - math.sin(math.rad(45))) < 1e-4, "a quarter turn about +Z")
```

`pc.asm.slider`: Put two axes on one line without turning: the body can only slide along it.

- `body` (id): The body that moves
- `face` (any): A round face, {axis = {point, direction}}, as pc.doc.faces lists it; an edge's line or circle axis goes the same way
- `other` (id): The body it is held against; the nil id (all zeros) for the world origin, its faces then in world space
- `other_face` (any): A round face, {axis = {point, direction}}, as pc.doc.faces lists it; an edge's line or circle axis goes the same way
- `name` (string, optional): Its name in the tree
- `drive` (any, optional): A hinge's angle (degrees from where it was made) or a slider's position (mm) to hold it at; false lets it move again
- `limits` (any, optional): {low, high}: the range a hinge's angle or a slider's position stays in while not driven; false takes the limits away
- Returns the joint's id

Notes:

- `body` moves and `other` stays. The document's first joint also grounds `other` (as `asm.ground` does), unless it is the world: the nil id, all zeros, whose faces are given in world space.
- Faces are given where the bodies sit now, in world space, as `pc.doc.faces` lists them; the joint keeps them in each body's own frame and solves as it is made. A joint that cannot hold with the others is not made: the call fails naming the joints in conflict.
- The axes go on one line and the body keeps one motion, the slide along it; it does not turn.
- `drive` holds the position in mm from where the slider was made, positive along the axis's direction; `limits` keeps it within {low, high}.

See also `pc.asm.set`, `pc.asm.travel`, `pc.asm.couple`, `pc.asm.align`.

Example: A carriage on a rail, 30 mm along.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local rail = box(0, 100, 10, 5)
local carriage = box(0, 20, 10, 5)
assert(#pc.doc.rebuild() == 0)
local below = {axis = {point = {0, 5, 0}, direction = {1, 0, 0}}}
local along = {axis = {point = {0, 5, 5}, direction = {1, 0, 0}}}
local s = pc.asm.slider{body = carriage, face = below,
  other = rail, other_face = along, limits = {0, 80}}
pc.asm.set{joint = s, drive = 30}
local at = pc.asm.placement{body = carriage}.translation
assert(math.abs(at[1] - 30) < 1e-3 and math.abs(at[3] - 5) < 1e-3,
  "on the rail, 30 mm along")
assert(math.abs(pc.asm.travel{joint = s} - 30) < 1e-3)
```

`pc.asm.fix`: Hold a body to another where it sits.

- `body` (id): The body that moves
- `face` (any, optional): Any face, as pc.doc.faces lists it; the body's origin when left out
- `other` (id): The body it is held against
- `other_face` (any, optional): Any face, as pc.doc.faces lists it; the body's origin when left out
- `name` (string, optional): Its name in the tree
- Returns the joint's id

Notes:

- `body` moves and `other` stays. The document's first joint also grounds `other` (as `asm.ground` does), unless it is the world: the nil id, all zeros, whose faces are given in world space.
- Faces are given where the bodies sit now, in world space, as `pc.doc.faces` lists them; the joint keeps them in each body's own frame and solves as it is made. A joint that cannot hold with the others is not made: the call fails naming the joints in conflict.
- With no faces it holds the body to the other where both sit now: nothing moves as it is made, and no motion is left.
- The fixed body follows the other at the next solve: `asm.move` and `asm.place` of the other do not carry it until then.

See also `pc.asm.set`, `pc.asm.group`, `pc.asm.ground`.

Example: A tag that follows its base.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local base = box(0, 20, 20, 5)
local tag = box(30, 5, 5, 5)
assert(#pc.doc.rebuild() == 0)
pc.asm.fix{body = tag, other = base}
assert(pc.asm.freedom{body = tag}[1].free == 0)
pc.asm.move{body = base, by = {0, 0, 10}}
pc.asm.solve{}
local at = pc.asm.placement{body = tag}.translation
assert(math.abs(at[3] - 10) < 1e-4, "the tag follows the base")
```

`pc.asm.parallel`: Keep two faces or axes parallel.

- `body` (id): The body that moves
- `face` (any): A flat face {point, normal} or a round face or edge {axis = {point, direction}}
- `other` (id): The body it is held against; the nil id (all zeros) for the world origin, its faces then in world space
- `other_face` (any): A flat face {point, normal} or a round face or edge {axis = {point, direction}}
- `name` (string, optional): Its name in the tree
- Returns the joint's id

Notes:

- `body` moves and `other` stays. The document's first joint also grounds `other` (as `asm.ground` does), unless it is the world: the nil id, all zeros, whose faces are given in world space.
- Faces are given where the bodies sit now, in world space, as `pc.doc.faces` lists them; the joint keeps them in each body's own frame and solves as it is made. A joint that cannot hold with the others is not made: the call fails naming the joints in conflict.
- It turns the body until the faces or axes are parallel and holds only that: the body may still slide every way and turn about the normal, four motions left.

See also `pc.asm.set`, `pc.asm.angle`, `pc.asm.perpendicular`, `pc.asm.mate`.

Example: A tilted plate turned back square.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local base = box(0, 20, 20, 5)
local plate = box(40, 10, 10, 2)
assert(#pc.doc.rebuild() == 0)
pc.asm.move{body = plate, turn = 20, axis = {0, 1, 0}}
local top = pc.doc.faces{body = plate}[6]
local base_top = pc.doc.faces{body = base}[6]
pc.asm.parallel{body = plate, face = top, other = base, other_face = base_top}
local w = pc.asm.placement{body = plate}.rotation[4]
assert(math.abs(w - 1) < 1e-6, "turned back square")
assert(pc.asm.freedom{body = plate}[1].free == 4,
  "it still slides every way and turns about Z")
```

`pc.asm.perpendicular`: Keep two faces or axes square to each other.

- `body` (id): The body that moves
- `face` (any): A flat face {point, normal} or a round face or edge {axis = {point, direction}}
- `other` (id): The body it is held against; the nil id (all zeros) for the world origin, its faces then in world space
- `other_face` (any): A flat face {point, normal} or a round face or edge {axis = {point, direction}}
- `name` (string, optional): Its name in the tree
- Returns the joint's id

Notes:

- `body` moves and `other` stays. The document's first joint also grounds `other` (as `asm.ground` does), unless it is the world: the nil id, all zeros, whose faces are given in world space.
- Faces are given where the bodies sit now, in world space, as `pc.doc.faces` lists them; the joint keeps them in each body's own frame and solves as it is made. A joint that cannot hold with the others is not made: the call fails naming the joints in conflict.
- It turns the body until the faces or axes are square to each other and holds only that, five motions left.

See also `pc.asm.set`, `pc.asm.parallel`, `pc.asm.angle`.

Example: A fin stood square to a base.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local base = box(0, 20, 20, 5)
local fin = box(40, 10, 10, 2)
assert(#pc.doc.rebuild() == 0)
pc.asm.move{body = fin, turn = 60, axis = {0, 1, 0}}
pc.asm.perpendicular{body = fin, face = pc.doc.faces{body = fin}[6],
  other = base, other_face = pc.doc.faces{body = base}[6]}
local n = pc.doc.faces{body = fin}[6].normal
assert(math.abs(n[3]) < 1e-4, "the fin's face stands square to the base's top")
assert(pc.asm.freedom{body = fin}[1].free == 5)
```

`pc.asm.distance`: Keep two faces, axes or points a distance apart: along a face, from an axis, between axes or points.

- `body` (id): The body that moves
- `face` (any): A flat face {point, normal}, a round face or edge {axis}, or a point ({centre} of a ball, or {point} alone)
- `other` (id): The body it is held against; the nil id (all zeros) for the world origin, its faces then in world space
- `other_face` (any): A flat face {point, normal}, a round face or edge {axis}, or a point ({centre} of a ball, or {point} alone)
- `name` (string, optional): Its name in the tree
- `offset` (number, optional): Along the second face's normal, mm; the distance they are now when left out
- Returns the joint's id

Notes:

- `body` moves and `other` stays. The document's first joint also grounds `other` (as `asm.ground` does), unless it is the world: the nil id, all zeros, whose faces are given in world space.
- Faces are given where the bodies sit now, in world space, as `pc.doc.faces` lists them; the joint keeps them in each body's own frame and solves as it is made. A joint that cannot hold with the others is not made: the call fails naming the joints in conflict.
- Between flat faces, `offset` is measured along the second face's normal, mm; left out, the distance they are apart now is kept.
- It holds only that distance: unlike a mate with an offset, the body may still tilt and slide, five motions left.
- Faces, axes `{axis}` and points `{centre}` or `{point}` mix: a face's distance from a point, an axis's from an axis.

See also `pc.asm.set`, `pc.asm.mate`.

Example: A plate held 10 mm above a base.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local function facing(body, z)
  for _, f in ipairs(pc.doc.faces{body = body}) do
    if f.normal and f.normal[3] * z > 0.99 then return f end
  end
end
local base = box(0, 20, 20, 5)
local plate = box(40, 10, 10, 2)
assert(#pc.doc.rebuild() == 0)
pc.asm.distance{body = plate, face = facing(plate, -1),
  other = base, other_face = facing(base, 1), offset = 10}
local at = pc.asm.placement{body = plate}.translation
assert(math.abs(at[3] - 15) < 1e-4, "10 mm above the base's top")
```

`pc.asm.tangent`: Rest a round face on a flat one.

- `body` (id): The body that moves
- `face` (any): A flat face {point, normal} on one body and a round face {axis, radius} on the other, either way round
- `other` (id): The body it is held against; the nil id (all zeros) for the world origin, its faces then in world space
- `other_face` (any): A flat face {point, normal} on one body and a round face {axis, radius} on the other, either way round
- `name` (string, optional): Its name in the tree
- `radius` (number, optional): The round face's radius, mm; the face's own when left out
- Returns the joint's id

Notes:

- `body` moves and `other` stays. The document's first joint also grounds `other` (as `asm.ground` does), unless it is the world: the nil id, all zeros, whose faces are given in world space.
- Faces are given where the bodies sit now, in world space, as `pc.doc.faces` lists them; the joint keeps them in each body's own frame and solves as it is made. A joint that cannot hold with the others is not made: the call fails naming the joints in conflict.
- One face is flat and the other round, either way round; two of one kind are refused.
- The round face's radius comes from the face, as `pc.doc.faces` lists it with `radius`, or from the `radius` argument; with neither the joint is refused.
- It leaves the body four motions.

See also `pc.asm.set`, `pc.asm.mate`, `pc.asm.cam`.

Example: A roller against a block's side.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 40, height = 40}
local base = pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
local r = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = r, x = 80, y = 0, radius = 3}
local roller = pc.doc.feature{id = pc.design.pad{sketch = r, length = 10}}.body
assert(#pc.doc.rebuild() == 0)
local side, round
for _, f in ipairs(pc.doc.faces{body = base}) do
  if f.normal and f.normal[1] > 0.99 then side = f end
end
for _, f in ipairs(pc.doc.faces{body = roller}) do
  if f.axis then round = f end
end
pc.asm.tangent{body = roller, face = round, other = base, other_face = side}
local at = pc.asm.placement{body = roller}.translation
assert(math.abs(80 + at[1] - 43) < 1e-3, "its axis 3 mm out from x = 40")
```

`pc.asm.ball`: Put two points together: the body can turn every way about them.

- `body` (id): The body that moves
- `face` (any): A point: a ball's {centre}, or {point} alone, as pc.doc.faces lists them
- `other` (id): The body it is held against; the nil id (all zeros) for the world origin, its faces then in world space
- `other_face` (any): A point: a ball's {centre}, or {point} alone, as pc.doc.faces lists them
- `name` (string, optional): Its name in the tree
- Returns the joint's id

Notes:

- `body` moves and `other` stays. The document's first joint also grounds `other` (as `asm.ground` does), unless it is the world: the nil id, all zeros, whose faces are given in world space.
- Faces are given where the bodies sit now, in world space, as `pc.doc.faces` lists them; the joint keeps them in each body's own frame and solves as it is made. A joint that cannot hold with the others is not made: the call fails naming the joints in conflict.
- It takes points: a ball's `{centre}`, else `{point}`; a flat face given whole is taken at its listed point.
- The two points meet and the body may turn every way, three motions left.

See also `pc.asm.set`, `pc.asm.universal`, `pc.asm.distance`.

Example: An arm's corner on a base's corner.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local base = box(0, 40, 40, 5)
local arm = box(60, 30, 4, 4)
assert(#pc.doc.rebuild() == 0)
pc.asm.ball{body = arm, face = {point = {60, 0, 4}},
  other = base, other_face = {point = {40, 40, 5}}}
local at = pc.asm.placement{body = arm}.translation
assert(math.abs(at[1] + 20) < 1e-4 and math.abs(at[2] - 40) < 1e-4
  and math.abs(at[3] - 1) < 1e-4, "the arm's corner on the base's corner")
assert(pc.asm.freedom{body = arm}[1].free == 3, "it turns every way")
```

`pc.asm.universal`: Cross two yokes' pins at one point, square to each other: the body turns about either.

- `body` (id): The body that moves
- `face` (any): A round face, {axis = {point, direction}}, as pc.doc.faces lists it; an edge's line or circle axis goes the same way
- `other` (id): The body it is held against; the nil id (all zeros) for the world origin, its faces then in world space
- `other_face` (any): A round face, {axis = {point, direction}}, as pc.doc.faces lists it; an edge's line or circle axis goes the same way
- `name` (string, optional): Its name in the tree
- Returns the joint's id

Notes:

- `body` moves and `other` stays. The document's first joint also grounds `other` (as `asm.ground` does), unless it is the world: the nil id, all zeros, whose faces are given in world space.
- Faces are given where the bodies sit now, in world space, as `pc.doc.faces` lists them; the joint keeps them in each body's own frame and solves as it is made. A joint that cannot hold with the others is not made: the call fails naming the joints in conflict.
- Each body gives a pin, `{axis = {point, direction}}`. The two axes' points meet and the body keeps two turns, one about each pin.

See also `pc.asm.set`, `pc.asm.ball`, `pc.asm.hinge`.

Example: Two pins crossed at one point.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local base = box(0, 40, 40, 5)
local arm = box(60, 30, 4, 4)
assert(#pc.doc.rebuild() == 0)
local along_x = {axis = {point = {60, 2, 2}, direction = {1, 0, 0}}}
local up_z = {axis = {point = {20, 20, 10}, direction = {0, 0, 1}}}
pc.asm.universal{body = arm, face = along_x, other = base, other_face = up_z}
local free = pc.asm.freedom{body = arm}[1]
assert(free.free == 2 and free.motions[1].turn and free.motions[2].turn)
local at = pc.asm.placement{body = arm}.translation
assert(math.abs(at[1] + 40) < 1e-3 and math.abs(at[3] - 8) < 1e-3,
  "the two pins cross at (20, 20, 10)")
```

`pc.asm.slot`: Keep a point on a line: a pin sliding in a slot.

- `body` (id): The body that moves
- `face` (any): The pin: a point ({centre} or {point}) on the moving body; the slot: a line {axis = {point, direction}} on the other
- `other` (id): The body it is held against; the nil id (all zeros) for the world origin, its faces then in world space
- `other_face` (any): The pin: a point ({centre} or {point}) on the moving body; the slot: a line {axis = {point, direction}} on the other
- `name` (string, optional): Its name in the tree
- Returns the joint's id

Notes:

- `body` moves and `other` stays. The document's first joint also grounds `other` (as `asm.ground` does), unless it is the world: the nil id, all zeros, whose faces are given in world space.
- Faces are given where the bodies sit now, in world space, as `pc.doc.faces` lists them; the joint keeps them in each body's own frame and solves as it is made. A joint that cannot hold with the others is not made: the call fails naming the joints in conflict.
- `face` is the pin, a point (`{centre}` or `{point}`) on the moving body; `other_face` the slot, a line `{axis = {point, direction}}` on the other.
- The pin stays on the line: the body keeps three turns and the slide along it.

See also `pc.asm.set`, `pc.asm.path`, `pc.asm.slider`.

Example: A pin kept on a line.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local base = box(0, 40, 40, 5)
local arm = box(60, 30, 4, 4)
assert(#pc.doc.rebuild() == 0)
local edge = {axis = {point = {0, 40, 5}, direction = {1, 0, 0}}}
local pin = {point = {60, 0, 0}}
pc.asm.slot{body = arm, face = pin, other = base, other_face = edge}
local at = pc.asm.placement{body = arm}.translation
assert(math.abs(at[2] - 40) < 1e-4 and math.abs(at[3] - 5) < 1e-4,
  "the pin on the base's back top edge")
local free = pc.asm.freedom{body = arm}[1]
assert(free.free == 4 and free.motions[4].slide[1] == 1,
  "three turns and a slide along X")
```

`pc.asm.path`: Keep a point on an edge of any shape: it runs along it.

- `body` (id): The body that moves
- `face` (any): The point that runs ({centre} or {point}); on the other body, a {point} on the edge it runs along
- `other` (id): The body it is held against; the nil id (all zeros) for the world origin, its faces then in world space
- `other_face` (any): The point that runs ({centre} or {point}); on the other body, a {point} on the edge it runs along
- `name` (string, optional): Its name in the tree
- Returns the joint's id

Notes:

- `body` moves and `other` stays. The document's first joint also grounds `other` (as `asm.ground` does), unless it is the world: the nil id, all zeros, whose faces are given in world space.
- Faces are given where the bodies sit now, in world space, as `pc.doc.faces` lists them; the joint keeps them in each body's own frame and solves as it is made. A joint that cannot hold with the others is not made: the call fails naming the joints in conflict.
- `other_face` is only a `{point}` near the edge: the other body's edge nearest it is taken, however far, and kept with the joint.
- The moving point goes onto that edge where the edge is nearest to it, not to the point given.

See also `pc.asm.set`, `pc.asm.slot`, `pc.asm.cam`.

Example: A corner run along a disc's rim.

```lua
local d = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = d, x = 0, y = 0, radius = 20}
local disc = pc.doc.feature{id = pc.design.pad{sketch = d, length = 5}}.body
local r = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = r, x = 60, y = 0, width = 4, height = 4}
local rider = pc.doc.feature{id = pc.design.pad{sketch = r, length = 4}}.body
assert(#pc.doc.rebuild() == 0)
pc.asm.path{body = rider, face = {point = {60, 0, 0}},
  other = disc, other_face = {point = {0, 20, 5}}}
local at = pc.asm.placement{body = rider}.translation
local x, y, z = 60 + at[1], at[2], at[3]
assert(math.abs(math.sqrt(x * x + y * y) - 20) < 0.01 and math.abs(z - 5) < 1e-3,
  "the rider's corner on the disc's top rim")
```

`pc.asm.cam`: Keep a follower on a cam's face, a roller's radius off it.

- `body` (id): The body that moves
- `face` (any): The follower ({centre} or {point}); on the other body, a {point} on the cam's face
- `other` (id): The body it is held against; the nil id (all zeros) for the world origin, its faces then in world space
- `other_face` (any): The follower ({centre} or {point}); on the other body, a {point} on the cam's face
- `name` (string, optional): Its name in the tree
- `radius` (number, optional): The follower's roller radius, mm; 0 for a point follower
- Returns the joint's id

Notes:

- `body` moves and `other` stays. The document's first joint also grounds `other` (as `asm.ground` does), unless it is the world: the nil id, all zeros, whose faces are given in world space.
- Faces are given where the bodies sit now, in world space, as `pc.doc.faces` lists them; the joint keeps them in each body's own frame and solves as it is made. A joint that cannot hold with the others is not made: the call fails naming the joints in conflict.
- `other_face` is a `{point}` near the cam's face: the other body's face nearest it is taken.
- `radius` is the roller's, mm: the follower's point keeps that far off the face; 0 when left out, a point follower.

See also `pc.asm.set`, `pc.asm.path`, `pc.asm.tangent`.

Example: A follower 2 mm off a round cam.

```lua
local d = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = d, x = 0, y = 0, radius = 20}
local cam = pc.doc.feature{id = pc.design.pad{sketch = d, length = 5}}.body
local r = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = r, x = 60, y = 0, width = 4, height = 4}
local follower = pc.doc.feature{id = pc.design.pad{sketch = r, length = 4}}.body
assert(#pc.doc.rebuild() == 0)
pc.asm.cam{body = follower, face = {point = {60, 0, 0}},
  other = cam, other_face = {point = {20, 0, 2}}, radius = 2}
local p = pc.asm.placement{body = follower}
local q, t = p.rotation, p.translation
-- Where the follower's point (60, 0, 0) is now: turned by q, then moved by t.
local x = 60 * (1 - 2 * (q[2] ^ 2 + q[3] ^ 2)) + t[1]
local y = 60 * 2 * (q[1] * q[2] + q[3] * q[4]) + t[2]
assert(math.abs(math.sqrt(x * x + y * y) - 22) < 0.1,
  "2 mm off the cam's 20 mm face")
```

`pc.asm.width`: Centre a tab's two faces between a slot's two walls.

- `body` (id): The body that moves
- `face` (any): A flat face, {point, normal}, as pc.doc.faces lists it
- `other` (id): The body it is held against; the nil id (all zeros) for the world origin, its faces then in world space
- `other_face` (any): A flat face, {point, normal}, as pc.doc.faces lists it
- `name` (string, optional): Its name in the tree
- `face2` (any): The tab's other flat face
- `other_face2` (any): The slot's other wall
- Returns the joint's id

Notes:

- `body` moves and `other` stays. The document's first joint also grounds `other` (as `asm.ground` does), unless it is the world: the nil id, all zeros, whose faces are given in world space.
- Faces are given where the bodies sit now, in world space, as `pc.doc.faces` lists them; the joint keeps them in each body's own frame and solves as it is made. A joint that cannot hold with the others is not made: the call fails naming the joints in conflict.
- It takes four flat faces: `face` and `face2`, the tab's two sides, and `other_face` and `other_face2`, the slot's two walls.
- The tab is centred between the walls and keeps three motions: two slides along the walls and a turn about their normal.

See also `pc.asm.set`, `pc.asm.mate`, `pc.asm.distance`.

Example: A tab centred between two walls.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 5, height = 20}
pc.sketch.rect{sketch = s, x = 15, y = 0, width = 5, height = 20}
local walls = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
local t = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = t, x = 40, y = 0, width = 6, height = 10}
local tab = pc.doc.feature{id = pc.design.pad{sketch = t, length = 10}}.body
assert(#pc.doc.rebuild() == 0)
local function side(body, sign, x)
  for _, f in ipairs(pc.doc.faces{body = body}) do
    local n = f.normal
    if n and n[1] * sign > 0.99 and math.abs(f.point[1] - x) < 1e-3 then
      return f
    end
  end
end
pc.asm.width{body = tab, face = side(tab, -1, 40), face2 = side(tab, 1, 46),
  other = walls, other_face = side(walls, 1, 5), other_face2 = side(walls, -1, 15)}
local at = pc.asm.placement{body = tab}.translation
assert(math.abs(at[1] + 33) < 1e-3, "the 6 mm tab centred in the 10 mm gap")
```

`pc.asm.couple`: Tie two joints' motions together: gears or a belt between two hinges, a rack and pinion or a screw between a hinge and a slider.

- `driver` (id): The hinge or slider that leads
- `driven` (id): The hinge or slider that follows
- `gearing` (string, optional): gears (hinges turning opposite ways), belt (the same way), rack (a hinge and a slider, by the pinion's pitch radius) or screw (by the lead); the first that suits the two joints when left out
- `ratio` (number, optional): Turns of the driven hinge per turn of the driver for gears and a belt, the pitch radius in mm for a rack, the lead in mm a turn for a screw
- `reverse` (boolean, optional): The driven joint moves the other way
- `name` (string, optional): Its name in the tree
- Returns the coupling's id

Notes:

- Both joints must be hinges or sliders. `gearing` left out is the first that suits them: gears for two hinges, a rack and pinion for a hinge and a slider.
- `ratio` is 1 when left out for gears and a belt, a 10 mm pitch radius for a rack and a 2 mm lead for a screw; it must be above zero.
- The tie starts where both joints stand as it is made. Moving either moves the other: `asm.turn` on the driven hinge turns the driver too.

See also `pc.asm.hinge`, `pc.asm.slider`, `pc.asm.set`.

Example: Two gears, the second half as fast the other way.

```lua
local function disc(x, r)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.circle{sketch = s, x = x, y = 30, radius = r}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = 3}}.body
end
local f = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = f, x = -50, y = 0, width = 100, height = 10}
local frame = pc.doc.feature{id = pc.design.pad{sketch = f, length = 2}}.body
local small, big = disc(0, 10), disc(30, 20)
assert(#pc.doc.rebuild() == 0)
local function at(x)
  return {axis = {point = {x, 30, 0}, direction = {0, 0, 1}}}
end
local h1 = pc.asm.hinge{body = small, face = at(0), other = frame, other_face = at(0)}
local h2 = pc.asm.hinge{body = big, face = at(30), other = frame, other_face = at(30)}
pc.asm.couple{driver = h1, driven = h2, ratio = 0.5}
pc.asm.set{joint = h1, drive = 40}
assert(math.abs(pc.asm.travel{joint = h2} + 20) < 1e-3,
  "gears: half as far, the other way")
```

`pc.asm.set`: Change a joint's gap, side, angle or radius, or a coupling's joints and ratio.

- `joint` (id): A joint or a coupling
- `gearing` (string, optional): gears (hinges turning opposite ways), belt (the same way), rack (a hinge and a slider, by the pinion's pitch radius) or screw (by the lead); the first that suits the two joints when left out
- `ratio` (number, optional): Turns of the driven hinge per turn of the driver for gears and a belt, the pitch radius in mm for a rack, the lead in mm a turn for a screw
- `reverse` (boolean, optional): A coupling's driven joint moves the other way
- `driver` (id, optional): A coupling's leading joint
- `driven` (id, optional): A coupling's following joint
- `offset` (number, optional): A mate's gap, a hinge's height or a distance, mm
- `flip` (boolean, optional): A mate's side
- `degrees` (number, optional): An angle joint's angle
- `radius` (number, optional): A tangent's radius, mm
- `drive` (any, optional): A hinge's angle (degrees from where it was made) or a slider's position (mm) to hold it at; false lets it move again
- `limits` (any, optional): {low, high}: the range a hinge's angle or a slider's position stays in while not driven; false takes the limits away
- `kind` (string, optional): A joint's new kind (mate, align, hinge, ...), from where the bodies stand
- `face` (any, optional): The moving body's face, picked afresh, as the joint's command takes it
- `other` (id, optional): The body it is held against, picked afresh
- `other_face` (any, optional): The other body's face, picked afresh
- `moving_end` (number, optional): How far the moving end sits along its own normal or axis, mm
- `fixed_end` (number, optional): How far the fixed end sits along its own normal or axis, mm
- `turn_drive` (any, optional): An alignment's turn (degrees from where it was made) to hold it at; false lets it turn
- `turn_limits` (any, optional): {low, high}: the range an alignment's turn stays in; false takes it away
- `slide_drive` (any, optional): How far along the axis (mm) to hold an alignment; false lets it slide
- `slide_limits` (any, optional): {low, high}: the range an alignment's slide stays in, mm; false takes it away

Notes:

- Only what is given changes; a setting the joint's kind does not have, such as `degrees` on a mate, is passed over without an error.
- `kind` makes the joint again as that kind from where the bodies stand, keeping its faces and its name; the new kind's settings start afresh (a mate at offset 0) unless given in the same call. A kind that does not take the faces is refused.
- `face`, `other` and `other_face` pick again, in world space as `pc.doc.faces` lists them. `moving_end` and `fixed_end` move each end of the joint along its own normal or axis, mm.
- Given a coupling, it changes `gearing`, `ratio`, `reverse`, `driver` and `driven`; a gearing that does not suit the joints is refused.

See also `pc.asm.couple`, `pc.asm.turn`.

Example: A mate's gap changed, then the mate made a distance.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local function facing(body, z)
  for _, f in ipairs(pc.doc.faces{body = body}) do
    if f.normal and f.normal[3] * z > 0.99 then return f end
  end
end
local base = box(0, 20, 20, 5)
local lid = box(40, 10, 10, 3)
assert(#pc.doc.rebuild() == 0)
local j = pc.asm.mate{body = lid, face = facing(lid, -1),
  other = base, other_face = facing(base, 1)}
local function height() return pc.asm.placement{body = lid}.translation[3] end
pc.asm.set{joint = j, offset = 2}
assert(math.abs(height() - 7) < 1e-4, "a 2 mm gap")
pc.asm.set{joint = j, kind = "distance", offset = 4}
assert(pc.doc.feature{id = j}.kind == "Distance")
assert(math.abs(height() - 9) < 1e-4, "4 mm apart")
```

`pc.asm.copy`: Insert linked copies of a body: each takes its shape and follows it, placed on its own.

- `body` (id): The body to copy
- `count` (number, optional): How many (1 when left out)
- `step` (list, optional): {x, y, z}: how far each copy sits from the one before, mm; beside it along X when left out
- `around` (any, optional): {point = {x, y, z}, direction = {x, y, z}, angle}: the copies turned about this axis instead, spread evenly over `angle` degrees (360 when left out)
- Returns the copies' ids

Notes:

- A copy has no features of its own: it takes the source's shape and follows its every rebuild, placed by its own placement.
- Left out, `step` puts each copy beside the one before along X, the body's width and a tenth apart (22 mm for a 20 mm body). `count` is held between 1 and 500.
- With `around`, a whole turn (360, the default) is shared with the source, so 3 copies stand at 90, 180 and 270 degrees; a part turn puts the last copy at `angle`. `around` needs a `direction`.

See also `pc.asm.mirror`, `pc.asm.parts`.

Example: Two copies in a row and a ring of three.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
assert(#pc.doc.rebuild() == 0)
local copies = pc.asm.copy{body = body, count = 2}
assert(#copies == 2)
local second = pc.asm.placement{body = copies[2]}.translation
assert(math.abs(second[1] - 44) < 1e-4, "22 mm apart along X")
pc.doc.set_value{id = pad, parameter = "length", value = 8}
assert(#pc.doc.rebuild() == 0)
local volume = pc.doc.measure{body = copies[1]}.volume
assert(math.abs(volume - 20 * 10 * 8) < 1e-3, "the copy follows")
local z = {point = {0, 0, 0}, direction = {0, 0, 1}}
local ring = pc.asm.copy{body = body, count = 3, around = z}
local w = pc.asm.placement{body = ring[1]}.rotation[4]
assert(math.abs(math.deg(2 * math.acos(w)) - 90) < 1e-3,
  "a whole turn shared by four")
```

`pc.asm.mirror`: Insert a linked copy that is a body's mirror image, following every change to it.

- `body` (id): The body to mirror
- `point` (list): A point of the mirror plane, {x, y, z}, in the world
- `normal` (list): The plane's normal, {x, y, z}
- Returns the mirrored copy's id

Notes:

- The plane is in world space. The copy keeps the identity placement and draws the source mirrored, following its changes.
- The mirrored mesh shows at once; its solid, which measuring and STEP export read, is made by `pc.doc.rebuild()`.
- A mirror of a mirrored copy is refused, as is a zero normal.

See also `pc.asm.copy`.

Example: A block mirrored across X = 0.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 10, y = 0, width = 20, height = 10}
local body = pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
assert(#pc.doc.rebuild() == 0)
local m = pc.asm.mirror{body = body, point = {0, 0, 0}, normal = {1, 0, 0}}
local faces = pc.doc.faces{body = m}
assert(#faces == 6)
for _, f in ipairs(faces) do
  local x = f.point[1]
  assert(x <= -10 + 1e-4 and x >= -30 - 1e-4, "on the other side of X = 0")
end
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = m}.volume - 1000) < 1e-6, "its solid, mirrored")
```

`pc.asm.replace`: Put another body in a body's place, with its joints found again on the new body's faces.

- `body` (id): The body to replace; it is hidden
- `with` (id): The body that takes its place
- Returns {kept, unmatched}: the joints whose ends were found on the new body, and those that were not

Notes:

- The replaced body is hidden, not deleted. Its joints move to the new body, found again on its faces, and the new body is placed where they put it.
- `kept` and `unmatched` list joint names. A body replacing itself is refused.

See also `pc.asm.copy`.

Example: A lid swapped for a thicker one.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local function facing(body, z)
  for _, f in ipairs(pc.doc.faces{body = body}) do
    if f.normal and f.normal[3] * z > 0.99 then return f end
  end
end
local base = box(0, 20, 20, 5)
local lid = box(40, 10, 10, 3)
local thicker = box(80, 12, 12, 4)
assert(#pc.doc.rebuild() == 0)
local j = pc.asm.mate{body = lid, face = facing(lid, -1),
  other = base, other_face = facing(base, 1)}
local report = pc.asm.replace{body = lid, with = thicker}
assert(#report.kept == 1 and #report.unmatched == 0)
assert(pc.doc.feature{id = j}.body == thicker, "the mate is on the new body")
local at = pc.asm.placement{body = thicker}.translation
assert(math.abs(at[3] - 5) < 1e-4, "on the base's top")
for _, b in ipairs(pc.doc.bodies()) do
  if b.id == lid then assert(not b.visible, "the old lid is hidden") end
end
```

`pc.asm.group`: Lock bodies together where they sit, in one rigid group.

- `bodies` (list): Two bodies or more; the first the one the rest hold to
- `group` (id, optional): A group to change to these bodies, rather than a new one
- `name` (string, optional): A new group's name in the tree
- Returns the group's id

Notes:

- The bodies hold to the first as they sit now, which leaves the others no motion. Fewer than two bodies are refused.
- `asm.move` and `asm.place` move one member only; the rest follow at the next solve. The bodies of a rigid `asm.component` move together at once.

See also `pc.asm.component`, `pc.asm.fix`.

Example: Two blocks locked together.

```lua
local function box(x)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = 5, height = 5}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
end
local a, b = box(0), box(10)
assert(#pc.doc.rebuild() == 0)
pc.asm.group{bodies = {a, b}}
assert(pc.asm.freedom{body = b}[1].free == 0, "b holds to a")
pc.asm.move{body = a, by = {0, 10, 0}}
assert(pc.asm.placement{body = b}.translation[2] == 0, "a move places one body")
pc.asm.solve{}
local at = pc.asm.placement{body = b}.translation
assert(math.abs(at[2] - 10) < 1e-4, "the solve brings b along")
```

`pc.asm.component`: Put bodies in a new component: one row in the tree that moves as one, or, flexible, keeps the joints inside it live; components nest.

- `bodies` (list): The bodies it holds, taken out of any other
- `name` (string, optional): Its name in the tree
- `parent` (id, optional): The component it sits in; the top if left out
- `flexible` (boolean, optional): The joints inside it move (false: rigid)
- Returns the component's id

Notes:

- Rigid, the default: `asm.move` or `asm.place` of any body in it moves every body in it and in the components nested in it. Flexible: each body moves alone.
- A body is taken out of any component it was in. Components are not features: `pc.doc.features` does not list them.

See also `pc.asm.component_set`, `pc.asm.component_add`, `pc.asm.group`.

Example: Two blocks that move as one.

```lua
local function box(x)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = 5, height = 5}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
end
local a, b, c = box(0), box(10), box(20)
assert(#pc.doc.rebuild() == 0)
pc.asm.component{bodies = {a, b}, name = "Pair"}
pc.asm.move{body = a, by = {0, 7, 0}}
assert(pc.asm.placement{body = b}.translation[2] == 7, "b moves with a")
assert(pc.asm.placement{body = c}.translation[2] == 0, "c is not in it")
```

`pc.asm.component_set`: Rename a component, move it, or make it rigid or flexible.

- `component` (id): The component
- `name` (string, optional): A new name
- `flexible` (boolean, optional): The joints inside it move
- `parent` (any, optional): The component it goes in, or "top" (or JSON null) for the top level

Notes:

- Only what is given changes. Made rigid again, the bodies move together from where they sit then.
- Lua has no null in a table, so `parent = "top"` takes a component out of the one it is in, to the top level.
- A component cannot be put inside itself; an unknown component is refused.

See also `pc.asm.component`, `pc.asm.component_add`.

Example: A component renamed, made flexible and rigid again.

```lua
local function box(x)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = 5, height = 5}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
end
local a, b = box(0), box(10)
assert(#pc.doc.rebuild() == 0)
local k = pc.asm.component{bodies = {a, b}}
pc.asm.component_set{component = k, name = "Loose pair", flexible = true}
pc.asm.move{body = a, by = {0, 7, 0}}
assert(pc.asm.placement{body = b}.translation[2] == 0, "flexible: a moves alone")
pc.asm.component_set{component = k, flexible = false}
pc.asm.move{body = a, by = {0, 1, 0}}
assert(pc.asm.placement{body = b}.translation[2] == 1, "rigid: they move as one")
```

`pc.asm.component_add`: Put bodies in a component, or take them out.

- `bodies` (list): The bodies
- `component` (id, optional): The component; left out, the bodies go to the top

Notes:

- A body is in one component at a time: adding it takes it out of the one it was in. Nothing moves.

See also `pc.asm.component`, `pc.asm.component_remove`.

Example: A body put in a component and taken out again.

```lua
local function box(x)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = 5, height = 5}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
end
local a, b, c = box(0), box(10), box(20)
assert(#pc.doc.rebuild() == 0)
local k = pc.asm.component{bodies = {a, b}}
pc.asm.component_add{bodies = {c}, component = k}
pc.asm.move{body = a, by = {0, 0, 3}}
assert(pc.asm.placement{body = c}.translation[3] == 3, "c moves with them")
pc.asm.component_add{bodies = {c}}
pc.asm.move{body = a, by = {0, 0, 3}}
assert(pc.asm.placement{body = c}.translation[3] == 3, "taken out, it stays")
```

`pc.asm.component_remove`: Take a component apart: its bodies and components go one level up.

- `component` (id): The component

Notes:

- Only the component goes: its bodies and nested components move one level up, and nothing moves in the model.

See also `pc.asm.component`.

Example: A component taken apart.

```lua
local function box(x)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = 5, height = 5}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
end
local a, b = box(0), box(10)
assert(#pc.doc.rebuild() == 0)
local k = pc.asm.component{bodies = {a, b}}
pc.asm.component_remove{component = k}
pc.asm.move{body = a, by = {0, 0, 3}}
assert(pc.asm.placement{body = b}.translation[3] == 0, "apart, a moves alone")
local renamed = pcall(pc.asm.component_set, {component = k, name = "Gone"})
assert(not renamed, "the component is gone")
```

`pc.asm.motion`: Keep a motion over time: hinges and sliders each driven by a formula of t, seconds.

- `drives` (list): {{joint = id, formula = "90 * t"}, ...}: a hinge's angle in degrees, a slider's position in mm
- `start` (number, optional): When it starts, s (0 when left out)
- `end` (number, optional): When it ends, s (2 when left out)
- `step` (number, optional): The time between frames, s (0.05 when left out)
- `study` (id, optional): A motion to change, rather than a new one
- `name` (string, optional): A new motion's name in the tree
- Returns the motion's id

Notes:

- Each formula gives its drive's value at time `t`, seconds: a hinge's angle in degrees, a slider's position in mm. A plain number takes the drive's unit; a formula that does not read, or gives a length for a hinge ("1 in * t") or an angle for a slider, is refused, and so is a joint that is not a hinge or a slider.
- `end` is a Lua keyword: write `["end"] = 1`. `start`, `end` and `step` are 0, 2 and 0.05 s when left out.
- Making it moves nothing: `asm.motion_frames` and `asm.trace` play it on a copy. `study` changes a motion already made, keeping its id.

See also `pc.asm.motion_frames`, `pc.asm.trace`, `pc.asm.motion_clashes`.

Example: A hinge turned 90 degrees a second, then changed.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
local a = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = a, x = -2, y = -2, width = 30, height = 4}
local arm = pc.doc.feature{id = pc.design.pad{sketch = a, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
local h = pc.asm.hinge{body = arm, face = z, other = post, other_face = z}
local m = pc.asm.motion{drives = {{joint = h, formula = "90 * t"}}, ["end"] = 1}
assert(#pc.asm.motion_frames{study = m} == 21, "0 to 1 s every 0.05 s")
local slower = {{joint = h, formula = "45 * t"}}
pc.asm.motion{study = m, drives = slower, ["end"] = 1, step = 0.5}
assert(#pc.asm.motion_frames{study = m} == 3, "the same motion, changed")
assert(pc.asm.travel{joint = h} == 0, "making it moves nothing")
```

`pc.asm.motion_frames`: Every body's placement at each frame of a motion; nothing is moved.

- `study` (id): The motion
- Returns a list of {t, bodies = {{body, translation, rotation}, ...}}

Notes:

- Frames run from `start` to `end`, both included, every `step`: 0 to 1 s every 0.25 s is 5 frames. Every body is in every frame.
- Each frame solves the joints with every drive held at its formula's value, on a copy: the bodies stay where they are. An id that is not a motion is refused ("is not a motion").

See also `pc.asm.motion`, `pc.asm.trace`.

Example: Where an arm is at the end of its motion.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
local a = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = a, x = -2, y = -2, width = 30, height = 4}
local arm = pc.doc.feature{id = pc.design.pad{sketch = a, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
local h = pc.asm.hinge{body = arm, face = z, other = post, other_face = z}
local drives = {{joint = h, formula = "90 * t"}}
local m = pc.asm.motion{drives = drives, ["end"] = 1, step = 0.25}
local frames = pc.asm.motion_frames{study = m}
local last = frames[#frames]
assert(#frames == 5 and last.t == 1)
for _, b in ipairs(last.bodies) do
  if b.body == arm then
    local turned = math.deg(2 * math.acos(b.rotation[4]))
    assert(math.abs(turned - 90) < 1e-3, "a quarter turn at 1 s")
  end
end
assert(pc.asm.placement{body = arm}.rotation[4] == 1, "the arm has not moved")
```

`pc.asm.trace`: Follow a point of a body through a motion: where it is and how fast at each frame.

- `study` (id): The motion
- `body` (id): The body
- `point` (list): {x, y, z} in the body's own frame
- Returns a list of {t, point, speed (mm/s)}

Notes:

- `point` is given in the body's own frame; each frame's `point` is where it is in the world then. Nothing moves. An id that is not a motion is refused ("is not a motion").

See also `pc.asm.motion_frames`, `pc.asm.motion`.

Example: The tip of a turning arm.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
local a = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = a, x = -2, y = -2, width = 30, height = 4}
local arm = pc.doc.feature{id = pc.design.pad{sketch = a, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
local h = pc.asm.hinge{body = arm, face = z, other = post, other_face = z}
local m = pc.asm.motion{drives = {{joint = h, formula = "90 * t"}}, ["end"] = 1}
local path = pc.asm.trace{study = m, body = arm, point = {28, 0, 0}}
local last = path[#path]
assert(math.abs(last.point[2] - 28) < 1e-3, "the arm's tip ends on +Y")
assert(math.abs(last.speed - 28 * math.pi / 2) < 0.1,
  "a quarter turn a second at 28 mm")
```

`pc.asm.exploded_view`: Keep an exploded view: steps, each moving some bodies by a shift, played in order.

- `steps` (list): {{bodies = {ids}, shift = {x, y, z}}, ...}, in the order they play
- `view` (id, optional): A view to change, rather than a new one
- `name` (string, optional): A new view's name in the tree
- Returns the view's id

Notes:

- Each step moves its bodies by `shift`, mm in world space, on from the steps before it; a step needs both `bodies` and `shift`.
- Making it moves nothing: `asm.explode_at` answers where it puts the bodies. `view` changes a view already made, keeping its id.

See also `pc.asm.explode_at`.

Example: A lid lifted, then the base moved aside.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local base = box(0, 20, 20, 5)
local lid = box(40, 10, 10, 3)
assert(#pc.doc.rebuild() == 0)
local view = pc.asm.exploded_view{steps = {
  {bodies = {lid}, shift = {0, 0, 20}},
  {bodies = {base}, shift = {-15, 0, 0}},
}}
for _, p in ipairs(pc.asm.explode_at{view = view, at = 2}) do
  if p.body == lid then assert(p.translation[3] == 20) end
  if p.body == base then assert(p.translation[1] == -15) end
end
assert(pc.asm.placement{body = lid}.translation[3] == 0, "nothing has moved")
```

`pc.asm.explode_at`: Where an exploded view puts every body, part way through its steps.

- `view` (id): The exploded view
- `at` (number): How many steps in: 1.5 is half way through the second
- Returns a list of {body, translation, rotation}; nothing is moved

Notes:

- Past the last step it stays at the end. Every body of the document is listed, moved by the view or not.

See also `pc.asm.exploded_view`.

Example: Half way through the second step.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
local lid = pc.doc.feature{id = pc.design.pad{sketch = s, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
local view = pc.asm.exploded_view{steps = {
  {bodies = {lid}, shift = {0, 0, 20}},
  {bodies = {lid}, shift = {10, 0, 0}},
}}
local at = pc.asm.explode_at{view = view, at = 1.5}[1].translation
assert(at[1] == 5 and at[3] == 20, "the first step done, half the second")
local past = pc.asm.explode_at{view = view, at = 9}[1].translation
assert(past[1] == 10, "past the last step is the last step")
```

`pc.asm.save_state`: Save where every body sits, which are hidden and where drives hold, under a name.

- `name` (string, optional): A new state's name in the tree
- `state` (id, optional): A saved state to keep the assembly in instead
- Returns the state's id

Notes:

- It keeps every body's placement, which bodies are hidden and the value each drive holds. `state` saves the assembly as it is now over a state already made, keeping its id.

See also `pc.asm.restore_state`.

Example: A state saved again after a change.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
local a = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = a, x = -2, y = -2, width = 30, height = 4}
local arm = pc.doc.feature{id = pc.design.pad{sketch = a, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
local h = pc.asm.hinge{body = arm, face = z, other = post, other_face = z,
  drive = 0}
local closed = pc.asm.save_state{name = "Closed"}
pc.asm.set{joint = h, drive = 90}
pc.asm.save_state{state = closed}
pc.asm.set{joint = h, drive = 10}
pc.asm.restore_state{state = closed}
assert(math.abs(pc.asm.travel{joint = h} - 90) < 1e-3, "saved again at 90")
```

`pc.asm.restore_state`: Put the assembly back as a saved state has it.

- `state` (id): The saved state

Notes:

- It puts back the placements, the hidden bodies (showing the rest) and the drives' values, then solves. Anything but a saved state is refused.

See also `pc.asm.save_state`.

Example: An opened arm put back, shown again.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
local a = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = a, x = -2, y = -2, width = 30, height = 4}
local arm = pc.doc.feature{id = pc.design.pad{sketch = a, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
local h = pc.asm.hinge{body = arm, face = z, other = post, other_face = z,
  drive = 0}
local closed = pc.asm.save_state{name = "Closed"}
pc.asm.set{joint = h, drive = 90}
pc.doc.set_visible{id = arm, visible = false}
pc.asm.restore_state{state = closed}
assert(math.abs(pc.asm.travel{joint = h}) < 1e-3, "the drive is back at 0")
for _, b in ipairs(pc.doc.bodies()) do
  if b.id == arm then assert(b.visible, "and the arm is shown again") end
end
```

`pc.asm.redundant`: The joints that hold nothing a body's other joints do not.

- Returns a list of {joint, name}

Notes:

- A joint is listed when its body's other joints already hold all it holds, such as a parallel beside a mate of the same faces. Nothing is removed.

See also `pc.asm.freedom`.

Example: A parallel that a mate makes needless.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local function facing(body, z)
  for _, f in ipairs(pc.doc.faces{body = body}) do
    if f.normal and f.normal[3] * z > 0.99 then return f end
  end
end
local base = box(0, 20, 20, 5)
local lid = box(40, 10, 10, 3)
assert(#pc.doc.rebuild() == 0)
pc.asm.mate{body = lid, face = facing(lid, -1),
  other = base, other_face = facing(base, 1)}
local p = pc.asm.parallel{body = lid, face = facing(lid, 1),
  other = base, other_face = facing(base, 1)}
local extra = pc.asm.redundant{}
assert(#extra == 1 and extra[1].joint == p and extra[1].name == "Parallel 1",
  "the mate already keeps them parallel")
```

`pc.asm.motion_clashes`: Step a hinge's or a slider's drive through a range and find where bodies collide.

- `joint` (id): The hinge or slider
- `low` (number): Where the steps start: degrees or mm
- `high` (number): Where they end
- `steps` (number, optional): How many steps (24 when left out)
- Returns a list of {at, a, b, volume (mm³)}: each step and pair sharing more material than where the joint stands

Notes:

- `steps` positions are checked, `low` and `high` among them: 3 from 0 to 180 are 0, 90 and 180. Each is solved on a copy: nothing moves.
- A pair is listed only where it shares more material than it does where the joint stands now, so a contact already there does not count. A joint that is not a hinge or a slider is refused.

See also `pc.asm.interference`, `pc.asm.motion`.

Example: An arm swung into a post.

```lua
local function box(x, y, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = y, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local base = box(-5, -5, 10, 10, 2)
local arm = box(-2, -2, 30, 4, 3)
local post = box(0, 15, 4, 4, 10)
assert(#pc.doc.rebuild() == 0)
local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
local h = pc.asm.hinge{body = arm, face = z, other = base, other_face = z}
local clashes = pc.asm.motion_clashes{joint = h, low = 0, high = 180, steps = 3}
assert(#clashes == 1 and clashes[1].at == 90,
  "the arm hits the post a quarter turn round")
assert(clashes[1].volume > 0)
assert(pc.asm.travel{joint = h} == 0, "the arm is left where it was")
```

`pc.asm.turn`: Turn a joint's body about the joint's axis or normal, the joint keeping it there.

- `joint` (id): The joint
- `degrees` (number): How far, degrees

Notes:

- On a hinge the angle moves on by `degrees`: a drive holding it moves with it, else the hinge is free again afterwards, where it was turned to. A hinge a coupling drives turns its driver to get there.
- Other joints turn the body about their normal or axis through the joint's point, the joint carried with it so it holds the body there. A ground is refused.

See also `pc.asm.flip`, `pc.asm.travel`.

Example: A mated block turned a quarter turn on its face.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local function facing(body, z)
  for _, f in ipairs(pc.doc.faces{body = body}) do
    if f.normal and f.normal[3] * z > 0.99 then return f end
  end
end
local base = box(0, 20, 20, 5)
local lid = box(5, 10, 4, 3)
assert(#pc.doc.rebuild() == 0)
local j = pc.asm.mate{body = lid, face = facing(lid, -1),
  other = base, other_face = facing(base, 1)}
pc.asm.turn{joint = j, degrees = 90}
local q = pc.asm.placement{body = lid}.rotation
assert(math.abs(math.deg(2 * math.acos(q[4])) - 90) < 1e-3, "a quarter turn")
assert(math.abs(q[3]) > 0.7, "about Z")
assert(math.abs(facing(lid, -1).point[3] - 5) < 1e-4, "still on the top")
```

`pc.asm.flip`: Turn a joint's body over, half a turn across the joint's axis or normal.

- `joint` (id): The joint

Notes:

- The joint is carried with the body, so it holds it turned over. On a mate that is `flip` in `asm.set`: a body resting on a face ends on its far side, inside the body it rested on.

See also `pc.asm.turn`, `pc.asm.set`.

Example: A wheel turned over on its hinge.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
local w = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = w, x = 30, y = 0, radius = 10}
local wheel = pc.doc.feature{id = pc.design.pad{sketch = w, length = 4}}.body
assert(#pc.doc.rebuild() == 0)
local function axis(x)
  return {axis = {point = {x, 0, 0}, direction = {0, 0, 1}}}
end
local h = pc.asm.hinge{body = wheel, face = axis(30),
  other = post, other_face = axis(0), offset = 3}
pc.asm.flip{joint = h}
local q = pc.asm.placement{body = wheel}.rotation
assert(math.abs(q[4]) < 1e-4, "half a turn")
assert(math.abs(q[3]) < 1e-4, "about an axis square to the hinge's")
```

`pc.asm.interference`: Where solid bodies share material: each pair that clashes, how much and where.

- `bodies` (list, optional): Only these bodies; every visible one when left out
- `clearance` (number, optional): Look instead for pairs nearer than this many mm
- Returns {checked, skipped, clashes, unchecked}, each clash {a, b, volume (mm³), centre}; skipped counts visible bodies with no solid, unchecked lists the pairs the kernel failed on as {a, b, error}. With a clearance, {checked, skipped, near, unchecked}, each near pair {a, b, distance (mm), on_a, on_b}, nearest first

Notes:

- Faces that only touch, a body resting on another, are no clash. `volume` is the shared material in mm³ and `centre` its middle, in world space.
- `clearance` answers the nearby pairs instead of the clashes, with the nearest point on each body; it takes longer.

See also `pc.asm.motion_clashes`, `pc.asm.mass`.

Example: A lid sunk 1 mm into its base.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local base = box(0, 20, 20, 5)
local lid = box(5, 10, 10, 3)
assert(#pc.doc.rebuild() == 0)
pc.asm.move{body = lid, by = {0, 0, 5}}
assert(#pc.asm.interference{}.clashes == 0, "resting on the top is no clash")
pc.asm.move{body = lid, by = {0, 0, -1}}
local found = pc.asm.interference{}
assert(found.checked == 2 and #found.clashes == 1)
local clash = found.clashes[1]
assert(math.abs(clash.volume - 10 * 10 * 1) < 1e-3, "1 mm of the lid sunk in")
assert(math.abs(clash.centre[3] - 4.5) < 1e-3)
```

`pc.asm.mass`: The mass and centre of mass of the solid bodies at one density.

- `bodies` (list, optional): Only these bodies; every visible one when left out
- `density` (number, optional): g/cm³ for bodies without a material (1 when left out)
- Returns {mass (g), volume (mm³), centre = {x, y, z} or nil, bodies = {{body, mass, volume, centre}, ...}, skipped}

Notes:

- `centre` is the centre of mass in world space, mm. An id in `bodies` that is not a body is passed over, so a list of feature ids answers a mass of 0.

See also `pc.asm.parts`, `pc.asm.interference`.

Example: A block's mass at 1.24 g/cm³.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 20}
local block = pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
assert(#pc.doc.rebuild() == 0)
local m = pc.asm.mass{density = 1.24}
assert(math.abs(m.volume - 2000) < 1e-3)
assert(math.abs(m.mass - 2.48) < 1e-6, "2 cm³ at 1.24 g/cm³")
assert(m.centre[1] == 10 and m.centre[3] == 2.5)
```

`pc.asm.parts`: Every part: bodies of the same shape counted together.

- `by_component` (boolean, optional): Each component's parts under it: every entry gains a depth, and components come as {component, name, depth}
- Returns a list of {name, quantity, bodies, size = {x, y, z} in mm or nil, mesh, number or nil, bought, values = {column = text}, print, volume (mm³) or nil, mass (g) or nil, density}, numbered parts first by number, then by name

Notes:

- Bodies count as one part when they share one shape: linked copies from `asm.copy` do; bodies modelled apart do not, however alike.
- `size` is the part's bounding box, mm.
- `volume` and `mass` are of one piece: the volume the kernel measures of its solid (a mesh body's from its triangles), the mass at its body's material's density, else at the list's printing material (`asm.print_material`). `print` is how many to print: the count set with `asm.part`, else one per body, none of a bought part.

See also `pc.asm.part`, `pc.asm.parts_table`, `pc.asm.copy`.

Example: A plate and its copies counted as one part.

```lua
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
```

`pc.asm.part`: Set what the parts list keeps for a part: its number, whether it is bought, its values in the added columns.

- `body` (id): Any body of the part
- `number` (number, optional): Its item number
- `bought` (boolean, optional): Bought rather than made: left out of exports and the slicer
- `values` (any, optional): {column = text}: its values, a column not yet in the list added to it
- `print` (number, optional): How many of it to print; below 0 goes back to one per body

Notes:

- What is set is kept for the whole part, every body of its shape, whichever body is named.
- A `number` of 0 or less takes the number away. A value that is not text is kept as text: 3 becomes "3".

See also `pc.asm.parts`, `pc.asm.parts_table`.

Example: A bought screw numbered on one of its copies.

```lua
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
```

`pc.asm.print_material`: Set the material the parts list weighs printed parts in.

- `name` (string): PLA, PETG, ABS, ASA, TPU, Nylon or PC, or a name of your own with its density
- `density` (number, optional): g/cm³; an offered material's own when left out
- Returns {name, density}

Notes:

- It is kept with the document. A body with a material of its own (`doc.set_body`) is weighed at that material's density instead.
- A name the list does not offer needs its density.

See also `pc.asm.parts`, `pc.asm.part`.

Example: A plate's filament in PETG, two to print.

```lua
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
```

`pc.asm.parts_table`: Replace what the parts list keeps, whole.

- `table` (any): {columns = {...}, entries = {[body id] = {number, bought, values}}}

Notes:

- It replaces the whole list: a number, bought mark or value the table leaves out is gone.
- An empty Lua table goes as a list and `entries` refuses it: leave `entries` out for none.

See also `pc.asm.part`, `pc.asm.parts`.

Example: The parts list written whole.

```lua
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
```

`pc.asm.travel`: Where a hinge or a slider has got to: the hinge's angle in degrees, the slider's position in mm.

- `joint` (id)
- Returns a number

Notes:

- It is read from where the bodies sit now, counted from where the joint was made, so a body moved by hand reads its new travel before any solve.
- Any other joint is refused, an alignment too.

See also `pc.asm.hinge`, `pc.asm.slider`, `pc.asm.turn`.

Example: A hinge's angle after its arm is moved.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
local a = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = a, x = -2, y = -2, width = 30, height = 4}
local arm = pc.doc.feature{id = pc.design.pad{sketch = a, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
local h = pc.asm.hinge{body = arm, face = z, other = post, other_face = z}
assert(pc.asm.travel{joint = h} == 0, "0 where it was made")
pc.asm.move{body = arm, turn = 45}
assert(math.abs(pc.asm.travel{joint = h} - 45) < 1e-3,
  "read from where the arm sits")
```

`pc.asm.ground`: Keep a body where it is: the bodies joined to it are placed against it.

- `body` (id)
- `grounded` (boolean, optional): false lets it move again (true by default)
- Returns the ground joint's id, or nil when it was taken away

Notes:

- Grounding a body already grounded answers its ground joint again. The document's first joint grounds the body its `other` names, so a ground is often there already.

See also `pc.asm.fix`, `pc.asm.freedom`.

Example: A base kept where it is.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local function facing(body, z)
  for _, f in ipairs(pc.doc.faces{body = body}) do
    if f.normal and f.normal[3] * z > 0.99 then return f end
  end
end
local base = box(0, 20, 20, 5)
local lid = box(40, 10, 10, 3)
assert(#pc.doc.rebuild() == 0)
local g = pc.asm.ground{body = base}
assert(pc.asm.ground{body = base} == g, "grounding twice keeps one ground")
pc.asm.mate{body = lid, face = facing(lid, -1),
  other = base, other_face = facing(base, 1)}
assert(pc.asm.placement{body = base}.translation[3] == 0, "the ground stays")
assert(pc.asm.ground{body = base, grounded = false} == nil)
```

`pc.asm.freedom`: What each jointed body may still do: the motions its joints leave open.

- `body` (id, optional): Only this body
- Returns a list of {body, free, motions}, each motion {turn = {axis, through}} or {slide = direction}, with at_limit true where a limit lets it go one way only

Notes:

- Grounded bodies, and bodies no joint moves, are not listed. A held drive takes its motion away; a limit does not: a hinge, slider or alignment resting on a limit keeps the motion, `at_limit`. A point on a path keeps its run along it at a corner too, and at an open path's end `at_limit`.
- `through` is a point on a turn's axis, in world space.

See also `pc.asm.redundant`, `pc.asm.solve`.

Example: What a mate leaves a lid.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local function facing(body, z)
  for _, f in ipairs(pc.doc.faces{body = body}) do
    if f.normal and f.normal[3] * z > 0.99 then return f end
  end
end
local base = box(0, 20, 20, 5)
local lid = box(40, 10, 10, 3)
assert(#pc.doc.rebuild() == 0)
pc.asm.mate{body = lid, face = facing(lid, -1),
  other = base, other_face = facing(base, 1)}
local all = pc.asm.freedom{}
assert(#all == 1 and all[1].body == lid, "the grounded base is not listed")
local turns, slides = 0, 0
for _, m in ipairs(all[1].motions) do
  if m.turn then turns = turns + 1; assert(m.turn.axis[3] == 1) end
  if m.slide then slides = slides + 1; assert(m.slide[3] == 0) end
end
assert(turns == 1 and slides == 2,
  "on a face: a turn about its normal and two slides along it")
```

`pc.asm.solve`: Place every body its joints hold.

- Returns what moved, in words

Notes:

- The joint commands, `asm.set` and `asm.ground` solve as they run; `asm.place` and `asm.move` do not, so solve after them.
- It answers "Moved 1 body; every joint holds", or "Every joint holds" when nothing had to move.
- A joint moves the body it belongs to; one that belongs to a grounded body moves the body at its other end instead. A joint between two bodies that cannot move (both grounded) and does not hold is refused, named ("cannot hold all its joints at once").

See also `pc.asm.place`, `pc.asm.move`.

Example: A lid put back on its base.

```lua
local function box(x, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local function facing(body, z)
  for _, f in ipairs(pc.doc.faces{body = body}) do
    if f.normal and f.normal[3] * z > 0.99 then return f end
  end
end
local base = box(0, 20, 20, 5)
local lid = box(40, 10, 10, 3)
assert(#pc.doc.rebuild() == 0)
pc.asm.mate{body = lid, face = facing(lid, -1),
  other = base, other_face = facing(base, 1)}
local function height() return pc.asm.placement{body = lid}.translation[3] end
pc.asm.place{body = lid, translation = {0, 0, 30}}
assert(height() == 30, "a placement does not solve")
assert(pc.asm.solve{} == "Moved 1 body; every joint holds")
assert(math.abs(height() - 5) < 1e-4)
assert(pc.asm.solve{} == "Every joint holds")
```

`pc.asm.placement`: Where a body sits.

- `body` (id)
- Returns {translation, rotation}, rotation a quaternion {x, y, z, w}

Notes:

- `translation` is in mm, in world space: where the body's own origin is, turned by `rotation`.

See also `pc.asm.place`, `pc.asm.move`.

Example: A body's placement before and after a move.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
local body = pc.doc.feature{id = pc.design.pad{sketch = s, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
local p = pc.asm.placement{body = body}
assert(p.translation[1] == 0 and p.rotation[4] == 1, "where it was modelled")
pc.asm.move{body = body, by = {5, 0, 0}, turn = 90}
p = pc.asm.placement{body = body}
assert(p.translation[1] == 5 and math.abs(p.rotation[3] - math.sqrt(0.5)) < 1e-6)
```

`pc.asm.place`: Put a body at a placement.

- `body` (id)
- `translation` (list, optional): {x, y, z} in mm
- `rotation` (list, optional): A quaternion {x, y, z, w}

Notes:

- It sets the placement outright; what is left out keeps its value. The quaternion is normalised, and a zero one is refused.
- It does not solve: joints catch up at the next `asm.solve`. The other bodies of a rigid component move with it.

See also `pc.asm.move`, `pc.asm.placement`, `pc.asm.solve`.

Example: A body lifted and turned.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
local body = pc.doc.feature{id = pc.design.pad{sketch = s, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
pc.asm.place{body = body, translation = {0, 0, 20}, rotation = {0, 0, 1, 1}}
local p = pc.asm.placement{body = body}
assert(p.translation[3] == 20)
local half = math.sqrt(0.5)
assert(math.abs(p.rotation[3] - half) < 1e-6, "the quaternion is normalised")
pc.asm.place{body = body, translation = {1, 2, 3}}
p = pc.asm.placement{body = body}
assert(math.abs(p.rotation[3] - half) < 1e-6, "the rotation left out is kept")
```

`pc.asm.move`: Move a body by a step and a turn.

- `body` (id)
- `by` (list, optional): {x, y, z} in mm
- `turn` (number, optional): Degrees about `axis`
- `axis` (list, optional): {x, y, z}; Z when left out
- `about` (list, optional): The point the turn is about, {x, y, z}; the origin when left out

Notes:

- It turns first, `turn` degrees about `axis` through `about`, then steps by `by`, mm in world space; both add to where the body is.
- It does not solve: joints catch up at the next `asm.solve`. The other bodies of a rigid component move with it.

See also `pc.asm.place`, `pc.asm.solve`.

Example: A plate turned about its own centre.

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 20}
local body = pc.doc.feature{id = pc.design.pad{sketch = s, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
pc.asm.move{body = body, turn = 90, about = {10, 10, 0}}
local t = pc.asm.placement{body = body}.translation
assert(math.abs(t[1] - 20) < 1e-4 and math.abs(t[2]) < 1e-4,
  "turned in place about its centre")
pc.asm.move{body = body, by = {0, 0, 5}}
t = pc.asm.placement{body = body}.translation
assert(math.abs(t[3] - 5) < 1e-4, "steps add up")
```
<!-- /commands -->
