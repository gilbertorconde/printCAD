# Sketcher query commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## sketch.geometry

A line's points are its start and end, an arc's its centre, start and end, a
circle's and an ellipse's their centre; positions are as last solved, with the
values formulas give its dimensions (what builds, which `pc.doc.feature{id =
s}.fields` may not yet be). Each element also says whether it is `external`.

Ends and centres are listed again as elements of kind point. A spline, a
parabola or a hyperbola is kind "other" with no points: its control points are
in `pc.doc.feature{id = s}.fields.sketch.geometry`.

See also: `sketch.constraints`, `doc.feature`

### Ends and centres are points of their own

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

## sketch.constraints

`kind` is the stored name (Coincident, Horizontal, Length, Diameter, Angle,
...): "distance" on a line lists as Length. `value` is a dimension's, angles
in degrees, as its formula sets it when it has one.

`items` are the element ids it ties. The origin and the axes show as fixed ids
ending in 0001 (origin), 0002 (x axis) and 0003 (y axis), not by name.

See also: `sketch.status`, `sketch.set_constraint`

### An angle listed in degrees

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

## sketch.regions

Curves split where they cross, touch or branch, and every closed area between
them is a region: two overlapping circles make three, the lens and two
crescents. Curves with a loose end are left out.

`name` stays while the sketch's dimensions change; a pad, pocket, revolution
or groove takes regions by it in its `regions` field. `x`, `y` is a point
inside the region, `area` in mm² with its holes taken out. `default` is true
for the regions a feature takes when it names none: a region inside another
loop is a hole, one inside that is material again.

See also: `design.pad`, `sketch.wall_thickness`

### Two crossing circles close three regions

```lua
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
```

## sketch.wall_thickness

It reads the sketch's closed profile, and refuses a sketch with none ("profile
is not closed").

`minimum` is the Sketcher preference when left out (0.8 mm unless changed);
`thin` is true when the thinnest wall is under it.

See also: `sketch.status`

### A frame with 1 mm walls

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
pc.sketch.rect{sketch = s, x = 1, y = 1, width = 18, height = 8}
local wall = pc.sketch.wall_thickness{sketch = s, minimum = 2.5}
assert(math.abs(wall.thinnest - 1) < 1e-4, "the frame's wall is 1 mm")
assert(wall.thin and wall.minimum == 2.5)
assert(not pc.sketch.wall_thickness{sketch = s, minimum = 0.5}.thin)
```

## sketch.status

`dof` is the freedom left: 0 is fully constrained. A free point has 2, a free
circle 3 (its centre and radius); a line's freedom is its two end points'.

`solved` false with ids in `conflicting` means constraints contradict; every
constraint taking part is listed, not only the newest. `redundant` lists those
that say again what others say; the sketch still solves. While `solved` is
false, what is built from the sketch fails at `pc.doc.rebuild()`.

See also: `sketch.constrain`, `sketch.constraints`

### A circle constrained, then over-constrained

```lua
local s = pc.sketch.new{plane = "XY"}
local c = pc.sketch.circle{sketch = s, x = 3, y = 4, radius = 5}
assert(pc.sketch.status{sketch = s}.dof == 3, "a centre that moves and a radius")
pc.sketch.constrain{sketch = s, kind = "radius", items = {c}, value = 5}
local centre = pc.sketch.geometry{sketch = s}[1].id
pc.sketch.constrain{sketch = s, kind = "lock", items = {centre}}
local status = pc.sketch.status{sketch = s}
assert(status.dof == 0 and status.solved and #status.conflicting == 0)
local other = pc.sketch.circle{sketch = s, x = 20, y = 4, radius = 6}
pc.sketch.constrain{sketch = s, kind = "equal", items = {c, other}}
pc.sketch.constrain{sketch = s, kind = "diameter", items = {other}, value = 12}
status = pc.sketch.status{sketch = s}
assert(not status.solved and #status.conflicting == 3, "radius 5, equal, diameter 12")
```
