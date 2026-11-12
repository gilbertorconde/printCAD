# Sketcher editing commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## sketch.solver_settings

`iterations` must be at least 1 and `tolerance` above 0 and below 1; either
may be left out to keep what the sketch has.

The sketch keeps them (`fields.sketch.solver` in `pc.doc.feature`) and is
solved again with them at once.

See also: `sketch.status`

### Settings kept on the sketch

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 5}
pc.sketch.solver_settings{sketch = s, iterations = 500, tolerance = 1e-6}
local solver = pc.doc.feature{id = s}.fields.sketch.solver
assert(solver.max_iterations == 500 and solver.tolerance == 1e-6)
assert(pc.sketch.status{sketch = s}.solved)
```

## sketch.repair

It is the answer to a profile that fails with "profile is not closed" because
ends miss by a hair, as a drawing brought in may. A Pad, Pocket, Revolution or
Groove made from such a sketch joins ends that near itself.

It answers in words, such as "2 end(s) joined, 1 duplicate curve(s) removed",
or "nothing to repair".

See also: `sketch.import_dxf`, `sketch.status`

### Ends that miss by microns joined

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 10, y2 = 0}
pc.sketch.line{sketch = s, x1 = 10.005, y1 = 0, x2 = 0, y2 = 10}
pc.sketch.line{sketch = s, x1 = 0, y1 = 10, x2 = 0, y2 = 0.003}
local said = pc.sketch.repair{sketch = s}
assert(said:find("2 end"), said)
local pad = pc.design.pad{sketch = s, length = 1}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.volume - 50) < 0.01, m.volume)
assert(pc.sketch.repair{sketch = s} == "nothing to repair")
```

## sketch.restore

`data` is the whole `fields` of `pc.doc.feature{id = s}`, plane included,
taken before the edits; a table that is not a sketch's data is refused.

See also: `doc.feature`

### Edits put back

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 5}
local saved = pc.doc.feature{id = s}.fields
pc.sketch.circle{sketch = s, x = 5, y = 2, radius = 1}
assert(#pc.sketch.geometry{sketch = s} == 10)
pc.sketch.restore{sketch = s, data = saved}
assert(#pc.sketch.geometry{sketch = s} == 8, "the circle and its centre are gone")
```

## sketch.delete

Deleting a point takes the curves that end or centre on it and their
constraints. Deleting a curve leaves its end points behind as loose points.

A constraint's id deletes only that constraint. The origin and the axes are
passed over; an id the sketch does not have is refused.

See also: `sketch.construction`, `sketch.repair`

### A corner deleted with the two lines on it

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

## sketch.construction

Construction geometry is left out of profiles: a construction circle inside an
outline cuts no hole. Constraints on it still hold.

`sketch.draw` makes construction geometry from the start with `construction =
true`.

See also: `sketch.draw`, `sketch.delete`

### A circle that guides, then cuts

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

## sketch.internal_geometry

Without `show` it switches: the same call twice makes the pieces and takes
them away again. Give `show = true` to be sure they are there.

An ellipse gets its major and minor axes with their ends and its two foci, all
construction held to it. Items naming no ellipse, parabola, hyperbola or
B-spline are refused.

See also: `sketch.draw`, `sketch.constrain`

### An ellipse's axes and foci shown and hidden

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

## sketch.section_view

A view setting only: nothing built from the sketch changes, and it shows only
while the sketch is open for editing in the window.

The sketch's data carries `section_view = true` while it is on.

### The setting kept on the sketch

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.section_view{sketch = s}
assert(pc.doc.feature{id = s}.fields.section_view == true)
pc.sketch.section_view{sketch = s, on = false}
assert(pc.doc.feature{id = s}.fields.section_view == nil)
```
