# Sketcher spline commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## sketch.to_bspline

Each curve is replaced and its id is gone; an arc's ends stay, as the spline's
first and last control points. Arcs and circles become rational splines,
exact.

A spline is kind "other" in `pc.sketch.geometry`; its control points, degree,
knots and weights are in `pc.doc.feature{id = s}.fields.sketch.geometry`.

It returns {elements, constraints}: the new control points and the spline.

See also: `sketch.join`, `sketch.spline_degree`

### A circle made a spline pads the same disc

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

## sketch.spline_degree

Only the sign of `by` counts: any number above 0 raises one degree, any below
lowers one, and 0 is refused.

Raising adds a control point. The bspline tool draws degree 3, which the data
leaves out: `degree` shows in it once changed.

See also: `sketch.spline_knots`, `sketch.to_bspline`

### A cubic raised to degree 4

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

## sketch.insert_knot

`at` need not lie on the curve: the knot goes at the parameter where the
spline passes nearest it.

Only a spline takes it; a line, arc or circle is refused ("is not a spline").

See also: `sketch.knot_multiplicity`, `sketch.spline_knots`

### A knot inserted halfway

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

## sketch.knot_multiplicity

`knot` is a value `sketch.spline_knots` lists; one where the spline has no
knot is refused.

A multiplicity above the degree is held at the degree. Asking for the one the
knot has is refused ("the knot is unchanged").

See also: `sketch.spline_knots`, `sketch.insert_knot`

### A knot doubled, then removed

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

## sketch.spline_knots

The knots at the ends are not listed: a spline fresh from the bspline tool
lists none.

An element that is not a spline answers an empty list rather than an error.

See also: `sketch.insert_knot`, `sketch.knot_multiplicity`

### A half circle as a spline has one double knot

```lua
local s = pc.sketch.new{plane = "XY"}
local arc = pc.sketch.arc{sketch = s, x = 0, y = 0, radius = 5, start = 0, ["end"] = 180}
local made = pc.sketch.to_bspline{sketch = s, items = {arc}}
local spline = made.elements[#made.elements]
local knots = pc.sketch.spline_knots{sketch = s, spline = spline}
assert(#knots == 1, "the knots at its ends are not listed")
assert(knots[1].knot == 0.5 and knots[1].multiplicity == 2)
```

## sketch.spline_weight

`point` is one of the spline's `control_points` in `pc.doc.feature`'s data,
where its `weights` stand in the same order; any other point is refused.

A weight of 0 or less is refused.

See also: `sketch.to_bspline`

### A control point weighed three times

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

## sketch.join

The curves are replaced by one spline ending where the chain ends; their ids
are gone.

It needs two or more curves meeting end to end in one chain: one curve, a gap
or a branch is refused. A sharp corner, such as a rectangle's, fails at the
default `tolerance` ("No spline follows these curves within 0.01 mm").

See also: `sketch.to_bspline`

### A line and a quarter arc made one spline

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
