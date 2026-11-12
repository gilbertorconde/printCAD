# Surface step commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

In an example, a line `{{name}}` stands for the setup in the section
headed `## {{name}}`.

## {{wave_start}}

An STL file of a wave sheet begun, `n` steps a side.

```lua
local path = os.tmpname() .. ".stl"
local f = io.open(path, "w")
f:write("solid wave\n")
local n = 30
```

## {{wave_scan}}

The wave's facets through `at(i, j)` written and imported as a scan, `scan`
its body.

```lua
local function facet(p, q, s)
  f:write("facet normal 0 0 0\nouter loop\n")
  for _, v in ipairs({p, q, s}) do
    f:write(string.format("vertex %.6f %.6f %.6f\n", v[1], v[2], v[3]))
  end
  f:write("endloop\nendfacet\n")
end
for i = 0, n - 1 do
  for j = 0, n - 1 do
    facet(at(i, j), at(i + 1, j), at(i + 1, j + 1))
    facet(at(i, j), at(i + 1, j + 1), at(i, j + 1))
  end
end
f:write("endsolid wave\n")
f:close()
pc.file.import{path = path, scan = true}
pc.doc.rebuild()
os.remove(path)
local scan
for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
```

## {{box_scan}}

A 20 by 10 by 6 box exported and imported as a scan, `scan` its body, the
box hidden.

```lua
local box = pc.doc.new_body{}
pc.design.primitive{body = box, kind = {Box = {length = 20, width = 10, height = 6}}}
assert(#pc.doc.rebuild() == 0)
local path = os.tmpname() .. ".stl"
pc.file.export{path = path, bodies = {box}}
pc.doc.set_visible{id = box, visible = false}
pc.file.import{path = path, scan = true}
pc.doc.rebuild()
os.remove(path)
local scan
for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
```

## surface.extrude

`length` is 10 mm when left out. `direction` is SketchNormal (square to the
first sketch), X, Y, Z or {Custom = {x, y, z}}; "Custom" without its numbers
is refused with the form it takes. `symmetric = true` runs half each way,
`reversed = true` the other way.

Built from edges (`curves`) it needs a direction of its own: SketchNormal
fails at rebuild ("the direction follows a sketch's plane").

An edge of another body (`BodyEdge`, the edge as `pc.doc.edges{body = id}`
lists it) is taken where that body sits, and the step builds again when that
body changes or moves.

See also: `surface.check`

### Two walls from an open outline

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

## surface.revolve

`angle_deg` is 360 when left out. `axis` is SketchVertical (the first sketch's
vertical axis through its origin), SketchHorizontal, X, Y, Z or {Custom =
{origin = {x, y, z}, direction = {x, y, z}}}.

A curve lying along the axis fails at rebuild ("the whole wire lies along the
axis, so it revolves out nothing").

See also: `surface.check`

### A cylinder's side from a line

```lua
local s = pc.sketch.new{plane = "XZ"}
pc.sketch.line{sketch = s, x1 = 5, y1 = 0, x2 = 5, y2 = 10}
local tube = pc.surface.revolve{sketches = {s}}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = tube}.body}
assert(math.abs(m.area - 2 * math.pi * 5 * 10) < 1e-3, m.area)
assert(math.abs(m.min[1] + 5) < 1e-6 and math.abs(m.max[3] - 10) < 1e-6)
```

## surface.planar

It fills closed flat loops, a loop inside another a hole. An open chain fails
at rebuild ("profile wire is not closed").

See also: `surface.sew`

### A plate with a hole

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

## surface.fill

The curves must meet end to end and close; ones that do not fail at rebuild
("the boundary does not close"). The loop may rise out of a plane, which
`surface.planar` cannot fill.

`continuity` (G0, G1 or G2) is how it meets the face of each edge given in
`boundary`; a sketch's curve is only touched.

See also: `surface.planar`

### A disc filling a circle

```lua
local s = pc.sketch.new{plane = "XY", offset = 20}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 5}
local cap = pc.surface.fill{sketches = {s}}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = cap}.body}
assert(math.abs(m.area - math.pi * 25) < 1e-3, m.area)
assert(math.abs(m.min[3] - 20) < 1e-6 and math.abs(m.max[3] - 20) < 1e-6)
```

## surface.ruled

Exactly two curves, the first sketch's and the second's; a third is refused.
Their ends pair start to start, as each was drawn: draw both the same way
round, or the sheet twists.

See also: `surface.loft`

### A slanted strip between two lines

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

## surface.loft

Two sections or more, a sketch each, passed through in order; `closed = true`
runs on from the last back to the first. Sections may differ: a square to a
circle gives a face per side.

`guides` shape the surface between the sections: each crosses every section
once, all in the same order, and the sections' ends or middles alike. A guided
loft is one face through sections of one curve each; it does not close ("a
loft that follows guides does not close"), and a guide missing a section fails
at rebuild.

See also: `surface.ruled`

### A bulge through three circles

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

## surface.sweep

`sketches` is the profile's sketch, then the path's. A closed path fails at
rebuild ("a sweep surface along a closed spine is not built"), which is what a
circle profile and its path given the other way round meet.

With `second_rail` the path is the first rail: the profile, an open chain,
runs from the path's start to the second rail's, and at every point along them
is turned and scaled so its ends ride the rails. Ends off the rails' starts
fail at rebuild ("the profile's ends must sit on the rails' starts").

See also: `surface.loft`

### A tube along a line

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

## surface.offset

`faces` are faces of the body as `pc.doc.faces` lists them (the entry itself
will do; a curved face has no `normal` and needs none). The copy is added
beside them, the faces kept. `distance` is 1 mm when left out.

See also: `doc.faces`, `surface.thicken`

### A sheet copied 3 mm up

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

## surface.extend

`edges` are free edges of the body's sheets, each {point = e.point, direction
= e.direction} from `pc.doc.edges`; each face grows past them by `length` (5
mm) and stays one face.

`continuity` G1 (the default) and G0 run straight on, G2 along the face's own
surface. Past a cylinder's straight side, a curved direction, only G2 builds;
G1 and G0 fail at rebuild ("a linear extension of an analytic surface across a
curved parameter line would leave the surface").

See also: `doc.edges`

### A sheet grown 5 mm past one side

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

## surface.blend

`first` and `second` are edges of the body's sheets, each {point, direction}
from `pc.doc.edges`; the blend is a new face between them, meeting each edge's
face with `continuity` (G1 by default, G0 or G2).

See also: `doc.edges`, `surface.fill`

### A bridge between two strips

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

## surface.split

`faces` are faces of the body as `pc.doc.faces` lists them. A sketch's curve
lands on the face as seen square to the sketch's plane, so a sketch above the
face cuts it too.

Each curve must cross the face from edge to edge; one ending inside fails at
rebuild ("it must run from boundary to boundary").

See also: `doc.faces`, `surface.trim`

### A sheet cut in two along a line

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

## surface.sew

It joins every face of the body where their edges meet; `gap` (mm) joins edges
up to that far apart too.

A shell that closes becomes a solid, with a volume. One that does not stays a
sheet, its faces now sharing edges: `surface.check` lists those joins and
`surface.fillet` can round them.

See also: `surface.check`, `surface.fillet`, `surface.planar`

### Two walls joined at their corner

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

## surface.fillet

`edges` are edges two faces of the body share, each {point, direction} from
`pc.doc.edges`; `radius` is 2 mm when left out.

Separate sheets that only touch share no edge: the round fails at rebuild
("Sew them first so they share it"). Sew them, or draw the faces from one
sketch.

See also: `surface.sew`, `doc.edges`

### Two walls rounded at their corner

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

## surface.fillet_faces

`first` and `second` are faces of the body as `pc.doc.faces` lists them, that
need share no edge: two separate surfaces. Each is cut back to where the round
touches it and the three are sewn into one sheet; `radius` is 2 mm when left
out.

The round rolls on the side each face's normal points to; `flip_first` or
`flip_second` rolls it on that face's other side. A side the ball cannot reach
both faces from fails at rebuild ("rounding between the faces failed").

See also: `surface.fillet`, `doc.faces`

### Two walls of an L that stop short of the corner, rounded together

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

## surface.thicken

Each sheet becomes a solid: `thickness` (2 mm) along the faces' normals,
negative the other way, or half each way with `both_sides = true`.

A body already sewn into a solid is refused at rebuild ("the body has no sheet
to thicken"): Design's Thickness hollows a solid.

See also: `surface.sew`, `surface.offset`

### A sheet given 2 mm

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

## surface.trim

It keeps what lies on the side the plane's normal points to: YZ keeps +X.
`flip = true` keeps the other side; `offset` moves the plane along its normal,
YZ with 4 being x = 4.

A plane that leaves nothing fails at rebuild ("the plane leaves nothing of the
body on the side kept; flip it or move it"). It cuts a sewn solid too, which
stays solid.

See also: `surface.split`, `surface.mirror`

### A sheet cut at x = 4, either side kept

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

## surface.mirror

It adds the body's reflection beside what the body holds, the original kept;
the two are separate pieces of the one body.

`plane` YZ (the default), XZ, XY or {Custom = {origin, normal}}, in the body's
frame; `offset` moves it along its normal: YZ with 10 reflects in x = 10.

See also: `surface.trim`

### A sheet and its reflection

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

## surface.fit_scan

A plane or cylinder is grown over the scan from `at` (the region its points
lie on within the fit's scatter) and fitted to it; the sheet covers what its
points cover, extended by `extend` of its size so the fits of neighbouring
faces cross and a trim between them can cut each to the other. A cylinder is a
whole turn. `sigma` is the fit's RMS distance from the points.

A free-form region is fitted with a B-spline by `pc.surface.fit_scan_spline`.

See also: `surface.fit_scan_spline`, `inspect.grow`, `surface.sew`

### A scanned box's top and side fitted as planes that cross past the edge

```lua
{{box_scan}}
local top = pc.surface.fit_scan{scan = scan, at = {10, 5, 6}}
local body = pc.doc.feature{id = top}.body
pc.surface.fit_scan{scan = scan, at = {20, 5, 3}, body = body, extend = 0.5}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
-- The top reaches past x = 20, the side above z = 6.
assert(m.max[1] > 22 and m.max[3] > 7, m.max[1])
```

## surface.trim_scan

Each of the body's sheets is cut along the curves where the others cross it;
every face it is cut into is then kept when `share` of its area lies within
`tolerance` of the scan's triangles, and what is kept is sewn. A shell that
closes becomes a solid. Fitted surfaces reach past their regions so that this
cutting meets them up.

See also: `surface.fit_scan`, `surface.sew`

### A scanned box's six sides fitted, trimmed by the scan into the box again

```lua
{{box_scan}}
local body
for _, at in ipairs({{10, 5, 6}, {10, 5, 0}, {0, 5, 3}, {20, 5, 3}, {10, 0, 3}, {10, 10, 3}}) do
  local f = pc.surface.fit_scan{scan = scan, at = at, body = body}
  body = pc.doc.feature{id = f}.body
end
pc.surface.trim_scan{body = body, scan = scan}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
assert(math.abs(m.volume - 1200) < 0.01, m.volume)
```

## surface.fit_scan_spline

The region spreads over the scan from the triangle at `at` across every edge
whose two triangles turn less than `turn_deg`; its points (up to 6000) are
kept with the step. A B-spline sheet is fitted to them, seeded on the plane or
cylinder they lie nearest and refined where it misses them by more than
`tolerance`, until it meets it or only noise is left.

See also: `surface.fit_scan`, `surface.trim_scan`

### A wavy sheet, scanned, fitted as one free-form surface

```lua
{{wave_start}}
local function at(i, j)
  local x, y = 20 * i / n, 20 * j / n
  return {x, y, 2 * math.sin(x / 4)}
end
{{wave_scan}}
local sheet = pc.surface.fit_scan_spline{scan = scan, at = {10, 10, 2 * math.sin(2.5)}}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = sheet}.body}
assert(math.abs(m.max[3] - 2) < 0.1 and math.abs(m.max[1] - 20) < 0.1, m.max[3])
```

## surface.fit_network

Its boundaries are numbered from 1 in order: every chain of each sketch in
`sketches`, then `curves`, then `polylines`. Each bounds one patch or two, and
two patches walk the one they share opposite ways. A patch's sides go round it
counter-clockwise seen from the side it faces: along u, up v, back along u,
back down v. A side may be 0, shrunk to the point its neighbours meet at: a
three-sided patch.

Every patch is fitted to the scan's points at once, those within the
boundaries' box, and the patches are sewn together: no gap between them,
tangent across every boundary but the creases, exactly where four patches meet
at a corner and nearly where three or five do. A patch gains spans where it
misses its points by more than `tolerance`.

Use `chained` for additional patches whose sides are lists of signed curve
numbers, in walking order; an empty list shrinks a side to a point. A side may
join several curves at T-junctions. Its opposite must have the same number of
curves, or one curve carrying the junctions to the next patch. The patch is
held C1 across each junction. `corrections` requests parameter correction
after refinement: each pass is kept only if RMS distance decreases.

See also: `surface.fit_scan_spline`, `surface.check`

### A wavy scan fitted as three patches meeting tangent at a T-junction

```lua
{{wave_start}}
local function z(x) return 2 * math.sin(x / 4) end
local function at(i, j)
  local x, y = 20 * i / n, 20 * j / n
  return {x, y, z(x)}
end
{{wave_scan}}
local function line(x1, y1, x2, y2)
  local pts = {}
  for k = 0, 20 do
    local x, y = x1 + (x2 - x1) * k / 20, y1 + (y2 - y1) * k / 20
    pts[#pts + 1] = {x, y, z(x)}
  end
  return pts
end
local sheet = pc.surface.fit_network{scan = scan,
  polylines = {line(0, 0, 10, 0), line(10, 0, 20, 0), line(20, 0, 20, 8),
    line(20, 20, 10, 20), line(10, 20, 0, 20), line(0, 20, 0, 0),
    line(10, 0, 10, 8), line(20, 8, 20, 20), line(10, 8, 10, 20),
    line(10, 8, 20, 8)},
  patches = {{2, 3, -10, -7}, {10, 8, 4, -9}},
  chained = {{{1}, {7, 9}, {5}, {6}}}, corrections = 2}
assert(#pc.doc.rebuild() == 0)
local body = pc.doc.feature{id = sheet}.body
local joins = pc.surface.check{body = body}
assert(#joins == 3)
for _, join in ipairs(joins) do
  assert(join.gap < 0.00001 and join.angle_deg < 0.01, "tangent at the junction")
end
local m = pc.doc.measure{body = body}
assert(math.abs(m.max[3] - 2) < 0.05 and math.abs(m.max[1] - 20) < 0.01, m.max[1] .. " " .. m.max[3])
```
