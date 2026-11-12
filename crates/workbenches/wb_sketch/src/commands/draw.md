# Sketcher drawing commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## sketch.point

A point the sketch already has exactly at (x, y) is returned rather than a
second one made.

Lines, arcs and circles drawn later with an end or a centre exactly on it take
it: making the points first is how a script knows the ids of the ends it
constrains.

See also: `sketch.constrain`, `sketch.geometry`

### A corner made first and shared

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

## sketch.line

Each end takes a point the sketch has exactly there, else a new one: lines
drawn end to end share their ends and close a profile.

Unlike `sketch.polyline` and `sketch.rect`, it adds no constraint: a level
line is not held level.

Its ends are points of their own: the line's `start` and `end` in
`pc.doc.feature{id = s}.fields.sketch.geometry`, or points made first with
`sketch.point`. Two ends at the same spot are refused ("a line needs two
different ends").

See also: `sketch.polyline`, `sketch.point`

### Three lines end to end close a triangle

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

## sketch.polyline

A point is `{x, y}` or `{x = .., y = ..}`. Ending on the first point closes
the outline as `closed = true` does: an end landing exactly on a point the
sketch has takes that point.

Only closed loops count in a profile: an open polyline is left out of what a
pad or pocket uses, and a sketch with no closed loop fails at
`pc.doc.rebuild()` with "profile is not closed", not when the feature is made.

See also: `sketch.rect`, `sketch.line`

### A closed triangle padded

```lua
local s = pc.sketch.new{plane = "XY"}
local lines = pc.sketch.polyline{sketch = s, points = {{0, 0}, {30, 0}, {0, 20}}, closed = true}
assert(#lines == 3)
local pad = pc.design.pad{sketch = s, length = 5}
assert(#pc.doc.rebuild() == 0, "the triangle closes")
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.volume - 30 * 20 / 2 * 5) < 1e-3)
```

## sketch.rect

(x, y) is a corner, not the centre: a rectangle centred on the origin starts
at (-width / 2, -height / 2). A negative width or height draws it to the left
or below.

Its sides are held level and upright but carry no dimensions;
`pc.sketch.constrain` adds them.

See also: `sketch.polyline`, `sketch.constrain`

### A plate centred on the origin

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

## sketch.circle

It takes the radius, not the diameter; a radius of 0 or less is refused.

A circle inside a closed outline of the same sketch is a hole in what is
padded from it; circles apart from each other pad as separate solids in one
body.

`pc.design.hole` reads only a circle's centre: the hole's size is its own
`diameter`, whatever the circle's radius.

See also: `design.hole`

### A washer: a ring padded from two circles

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 10}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 4}
local pad = pc.design.pad{sketch = s, length = 2}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.volume - math.pi * (100 - 16) * 2) < 0.01, m.volume)
```

## sketch.arc

Angles are degrees, and the arc runs counter-clockwise from `start` to `end`:
-90 to 90 is the right half, 90 to -90 the left.

It takes the radius, not the diameter; a radius of 0 or less is refused.

Its centre and both ends are points of their own, each taking a point the
sketch has exactly there, so a line drawn to an end joins it.

See also: `sketch.circle`, `sketch.draw`

### A half disc closed by a line

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

## sketch.draw

Each click is in the sketch's millimetres and snaps as a click in the view
does: one on the origin is held there and one on an axis is held on it, the
constraints coming with what is made.

A tool takes the clicks its shape needs: line, circle (centre, then a point on
it) and slot (the ends of its centre line) two; rect_center its centre, then a
corner; ellipse its centre, an end of the major axis, then a point on it.
Clicks short of a shape make nothing, without an error.

Sizes not clicked come from `params`: `slot_width` 4 mm, `polygon_sides` 6,
`fillet_radius` and `chamfer_length` 2 mm when not given. A fillet or chamfer
takes one click on the corner. A bspline ends with the word "finish" in
`points`.

A value typed at a click (`typed = {length = 20}`) sets the shape's size; with
`constrain = true` it is kept as a dimension. The names are length, angle,
width, height, diameter, radius, major_radius, minor_radius, sweep (an arc's
turn, degrees counter-clockwise) and x and y (a shape's first point).

It returns every element made, end points and centres included, and every
constraint made with them.

See also: `sketch.polyline`, `sketch.rect`, `sketch.circle`

### A slot drawn by its centre line

```lua
local s = pc.sketch.new{plane = "XY"}
local made = pc.sketch.draw{sketch = s, tool = "slot", points = {{0, 0}, {20, 0}}, params = {slot_width = 6}}
assert(#made.elements > 0 and #made.constraints > 0)
local pad = pc.design.pad{sketch = s, length = 1}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.volume - (20 * 6 + math.pi * 3 ^ 2)) < 1e-3, m.volume)
```

## sketch.drag

`by` is a step {dx, dy} in mm, not a place to go to: the items move by it as
far as their constraints let them, and the rest of the sketch follows.

A point held by "lock" stays where it is. A corner of a rectangle from
`sketch.rect` stretches it, the opposite corner staying put.

Dragging a text block's point moves the whole text.

See also: `sketch.set_value`

### A rectangle stretched by its corner

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

## sketch.array

The items are the first copy: `rows = 2, cols = 3` makes five more. Columns
step along the sketch's x by `dx`, rows along its y by `dy`.

`rows` and `cols` must be at least 1, and one of them more than 1 ("an array
needs elements and at least two rows or columns").

Without `linked` the copies are free geometry; with it, constraints keep them
the originals' size and on the pitch.

It returns {elements, constraints}: the copies, their points and centres
included.

See also: `sketch.draw`, `design.linear_pattern`

### A plate with six holes in two rows

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

## sketch.paste

`clipboard` is a sketch's own data: `pc.doc.feature{id = s}.fields.sketch` of
any sketch serves, and all of its geometry comes, with its constraints.

`by` is a step {dx, dy} in mm. It returns {elements, constraints}.

See also: `sketch.carbon_copy`

### A rectangle pasted beside itself

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
