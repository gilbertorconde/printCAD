# Surface check commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## surface.check

One entry per edge two faces of the body share: `gap` in mm between them,
`angle_deg` the crease (0 where they meet tangent), `point` halfway along the
edge. Any body with a shape is checked, a Design solid too.

`curvature` is the largest difference, in 1/mm, between how sharply the two
faces bend square to the edge (nil where it could not be read); `join` is how
the view labels it: G2 where they meet tangent and bend alike, G1 tangent,
else the crease angle, or the gap where they are apart.

Sheets that only touch share no edge until sewn, so the seam between them is
not listed: two separate sheets give an empty list. A body not built yet is
refused ("The body has no shape to check yet").

See also: `surface.sew`, `surface.fillet`

### A sharp corner, then rounded

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

## surface.curvature

`mean`, `max` and `min` are signed against the faces' outward normals:
negative where a surface bulges out, positive in a hollow. `gaussian` is the
product of the two principal curvatures (1/mm²): positive on a dome or in a
bowl, negative on a saddle, zero on a plane and on what unrolls flat (a
cylinder, a cone). `mean` is their average, `max` and `min` each (1/mm).

The view paints the body until the task closes, or until `surface.zebra` or
`surface.check` takes its place. A body not built yet is refused ("The body
has no shape to paint yet").

See also: `surface.zebra`, `surface.check`

### A tube's side bends one way only

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

## surface.zebra

Each stripe is a band of the way the surface faces, turned about `axis`.
Across a crease the stripes break, across a tangent join they bend sharply,
and across a curvature continuous one they run on smoothly. The view paints
the body until the task closes.

See also: `surface.curvature`, `surface.check`

### Stripes on a tube

```lua
local s = pc.sketch.new{plane = "XZ"}
pc.sketch.line{sketch = s, x1 = 5, y1 = 0, x2 = 5, y2 = 10}
local body = pc.doc.feature{id = pc.surface.revolve{sketches = {s}}}.body
assert(#pc.doc.rebuild() == 0)
assert(pc.surface.zebra{body = body, stripes = 8} == nil)
local ok = pcall(pc.surface.zebra, {body = body, axis = "W"})
assert(not ok, "X, Y or Z")
```
