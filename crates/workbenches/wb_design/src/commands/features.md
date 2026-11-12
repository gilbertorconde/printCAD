# Design feature commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## design.pad

`length` is 10 mm when left out. The pad grows along the sketch's normal;
`reversed = true` grows it the other way, `symmetric = true` half each way.

It goes in the sketch's body. With no sketch, `face_point` and `face_normal`
name a flat face of the solid to extrude instead.

`regions` takes some of the sketch's regions by name (`sketch.regions`), where
its curves cross or branch: neighbours merge, and one left out inside a taken
one is a hole. Left out, the pad takes the whole profile.

See also: `sketch.new`, `sketch.regions`, `design.pocket`

### The lens two crossing circles share, padded alone

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 10}
pc.sketch.circle{sketch = s, x = 10, y = 0, radius = 10}
local lens
for _, r in ipairs(pc.sketch.regions{sketch = s}) do
  if math.abs(r.x - 5) < 1 then lens = r.name end
end
local pad = pc.design.pad{sketch = s, length = 5, regions = {lens}}
assert(#pc.doc.rebuild() == 0, "the lens pads")
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
local area = 2 * 100 * math.acos(0.5) - 5 * math.sqrt(300)
assert(math.abs(m.volume - 5 * area) < 1e-3, m.volume)
assert(math.abs(m.min[1]) < 1e-6 and math.abs(m.max[1] - 10) < 1e-6)
```

### A plate padded from a rectangle

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 40, height = 30}
local pad = pc.design.pad{sketch = s, length = 5}
assert(#pc.doc.rebuild() == 0, "the pad builds")
local body = pc.doc.feature{id = pad}.body
assert(math.abs(pc.doc.measure{body = body}.volume - 40 * 30 * 5) < 1e-3)
assert(#pc.doc.faces{body = body} == 6, "a box has six faces")
```

## design.pocket

`keep_inside = true` keeps only what the pocket's prism shares with the body
instead of cutting it away (Groove, and a cutting Loft, Pipe or primitive take
it too); a pattern cannot repeat such a feature.

It cuts against its sketch's normal: from a sketch at the top face's height it
digs down into the solid. From a sketch on the bottom (XY at 0 under a pad) it
cuts away from the material and removes nothing, without an error; `reversed =
true` turns it.

`through_all = true` cuts through everything; else `depth`, 5 mm when left
out.

It is refused in a body with no solid feature yet. Its sketch must be in the
padded body: `pc.sketch.new{body = ...}`, since a sketch made without `body`
starts a new one.

See also: `sketch.new`, `design.hole`

### A square window cut through a plate

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

## design.revolve

It turns the sketch about the sketch's own y axis through its origin (`axis =
"SketchY"`; on an XZ sketch that is the world's Z), 360 degrees when
`angle_deg` is left out; `axis = "SketchX"` or `axis = {Custom = {origin = {x,
y}, dir = {x, y}}}` in sketch coordinates turn it about another.

A profile that touches the axis makes a solid of revolution; one that crosses
it fails at `pc.doc.rebuild()` (the profile sits on the axis).

An edge, datum line or body axis taken as the axis must run along the sketch
plane; with `project_axis = true` one tilted to it turns about its shadow on
the plane, and only one square to it is refused. Groove and helix take it too.

It is refused without a sketch.

See also: `design.groove`, `design.helix`

### A tube turned from a rectangle beside the axis

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

## design.groove

It takes away what `design.revolve` would add: turned about the y axis through
the origin, 360 degrees; `axis` and `angle_deg` are the same fields.

It is refused where no solid is built yet, and its profile must be drawn in
the same history as that solid.

See also: `design.revolve`, `design.pocket`

### A ring groove cut into a turned shaft

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

## design.loft, design.subtractive_loft

`sections = {a, b, ...}` lists the sketches in order; given only `sketch`, the
loft has that one section and fails at `pc.doc.rebuild()` (a loft needs at
least two sections).

It goes in the first section's body; later sections may be sketches of other
bodies. A sketch of a single point as the first or last section closes the
loft to a tip.

`ruled = true` joins the sections with straight walls; else the walls run
smoothly through them. `closed = true` loops back to the first.

## design.loft

See also: `design.subtractive_loft`, `design.pipe`

### A square frustum lofted between two rectangles

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

## design.subtractive_loft

It is refused in a body with no solid feature yet.

See also: `design.loft`

### A conical dimple: a circle lofted down to a point and cut

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

## design.pipe, design.subtractive_pipe

`sketch` is the profile. The path is `spine`, a sketch id; left out, it is the
latest other sketch of the profile's body. The profile sits at either end of
the path, and the pipe runs from there.

`spine` may be a sketch of any body. Left out, it is refused when the
profile's body has no other sketch.

A path's sharp corners are mitred (`corner = "Transformed"`); `orientation`
sets how the profile turns along it.

`path_curves`, ids of the path sketch's curves, makes the path of those alone,
one chain; left out, the path is every curve the sketch draws. The toolbar's
Pipe with curves of a sketch picked in the view takes that sketch's picked
curves as the path and the body's latest other sketch as the profile.

## design.pipe

See also: `design.subtractive_pipe`, `sketch.polyline`

### A rod swept up and across along a bent path

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

### A rod along two sides of a rectangle, its other sides left out

```lua
local profile = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = profile, x = 20, y = 0, radius = 2}
local body = pc.doc.feature{id = profile}.body
local path = pc.sketch.new{body = body, plane = "XZ"}
local sides = pc.sketch.rect{sketch = path, x = 0, y = 0, width = 20, height = 10}
-- The right side and the top: up 10 mm from the profile, then across 20.
local pipe = pc.design.pipe{sketch = profile, spine = path,
                            path_curves = {sides[2], sides[3]}}
assert(#pc.doc.rebuild() == 0, "the pipe builds")
assert(#pc.doc.feature{id = pipe}.fields.Pipe.path_curves == 2)
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - math.pi * 4 * 30) < 1e-3, volume)
```

## design.subtractive_pipe

It is refused in a body with no solid feature yet.

See also: `design.pipe`

### A half-round channel cut along the top of a block

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

## design.helix, design.subtractive_helix

It sweeps the sketch along a helix about the sketch's y axis through its
origin (`axis`, as `design.revolve` takes it). `mode` says which two of
`pitch`, `height` and `turns` count: PitchHeight (the default: 5 mm pitch, 20
mm high; `turns` is then ignored), PitchTurns, HeightTurns, or
HeightTurnsGrowth with `growth` per turn.

`left_handed`, `cone_angle_deg` and `reversed` are fields too.

## design.helix

See also: `design.subtractive_helix`, `design.revolve`

### A square section swept a quarter turn

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

## design.subtractive_helix

It is refused in a body with no solid feature yet. `keep_inside = true` keeps
what the sweep shares with the body instead of cutting it.

See also: `design.helix`, `design.hole`

### A thread-like groove cut a quarter turn into a cylinder

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

## design.primitive, design.subtractive_primitive

It takes no sketch, so `body` is required. Left as it comes, a shape is about
10 mm across and placed at the body's origin: a box from 0 to 10 along each
axis, a cylinder of radius 5 standing 10 tall on it. A `variant` other than
the shapes listed is refused.

`kind` replaces the shape whole: `kind = {Cylinder = {radius = 3, height = 8,
angle_deg = 360}}`, every field of it given, else it is refused (missing
field). `pc.doc.feature{id = ...}` shows a shape's fields.

`placement = {origin = {x, y, z}, x_axis = {..}, z_axis = {..}}` places it in
the body's frame; an `x_axis` along the `z_axis` fails at `pc.doc.rebuild()`.

## design.primitive

See also: `design.subtractive_primitive`, `doc.new_body`

### A box with a cylinder standing on it

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

### A tube, and one whose inner radius is not below its outer refused

```lua
local body = pc.doc.new_body{}
local tube = pc.design.primitive{body = body, variant = "tube",
  kind = {Tube = {outer = 10, inner = 8, height = 20}}}
assert(#pc.doc.rebuild() == 0)
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - math.pi * (100 - 64) * 20) < 1e-3, volume)
pc.design.set{feature = tube, kind = {Tube = {outer = 10, inner = 10, height = 20}}}
assert(#pc.doc.rebuild() == 1, "an inner radius as large as the outer is refused")
```

## design.subtractive_primitive

It is refused in a body with no solid feature yet.

See also: `design.primitive`, `design.hole`

### A cylinder cut through the default box

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

## design.hole

It drills at every circle's centre and every point of its sketch; the circles'
sizes are ignored, the hole's own `diameter` (5 mm when left out) is what it
drills.

It drills against the sketch's normal, `depth` deep (10 mm) or `through_all =
true`. A sketch on the bottom of a pad drills away from the material and
removes nothing, without an error; `reversed = true` turns it.

Counterbores, countersinks and threads are fields too (`cut`, `threaded`,
`thread`); docs/HOLES.md describes them, and `pc.doc.feature{id = ...}` shows
a hole's fields.

`nut_trap = true` cuts a hexagonal pocket for a captive nut at the hole's
mouth, sized from the ISO 4032 nut of its metric `thread` with 0.3 mm
clearance; give a table for more: `{side = "Bottom", clearance, depth,
across_flats, turn_deg, standard = "Din934"}`. At the bottom it ends where a
hole with a `depth` ends; through all, it is refused.

See also: `sketch.circle`, `design.pocket`

### An M3 clearance hole with a nut trap at its mouth

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

### Two holes through a plate, one at a circle and one at a point

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 40, height = 20}
local pad = pc.design.pad{sketch = s, length = 4}
local body = pc.doc.feature{id = pad}.body
local at = pc.sketch.new{body = body, plane = "XY", offset = 4}
pc.sketch.circle{sketch = at, x = 10, y = 10, radius = 1}
pc.sketch.point{sketch = at, x = 30, y = 10}
pc.design.hole{sketch = at, diameter = 6, through_all = true}
assert(#pc.doc.rebuild() == 0, "the holes build")
local bores = 0
for _, face in ipairs(pc.doc.faces{body = body}) do
  if face.kind == "cylinder" then
    bores = bores + 1
    assert(math.abs(face.radius - 3) < 1e-6, "the hole's diameter, not the circle's")
  end
end
assert(bores == 2)
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (40 * 20 - 2 * math.pi * 9) * 4) < 1e-3, volume)
```

## design.fillet, design.chamfer

`edges = {Edges = {{point = {x, y, z}, direction = {x, y, z}}, ...}}` picks
edges by a point on each and its direction there, and `edges = {Faces =
{{point = .., normal = ..}}}` every edge around those faces, in the body's own
frame. A bare list of picks is refused.

Edges running on tangentially are taken too (`follow_tangent`, on by default).

See also: `doc.faces`

## design.fillet

A radius larger than the faces beside an edge can take fails at
`pc.doc.rebuild()`.

See also: `design.chamfer`

### One top edge of a block rounded

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

## design.chamfer

`mode` is EqualDistance, TwoDistances (with `size2`) or DistanceAngle (with
`angle_deg`).

See also: `design.fillet`

### The top face's edges bevelled

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

## design.draft

`face_point` and `face_normal` name the neutral plane's face and are required;
`faces = {{point = .., normal = ..}, ...}` are the faces to tilt. Without
`faces` it fails at `pc.doc.rebuild()` (no faces to tilt).

`angle_deg` is 1.5 when left out. The faces keep their place on the neutral
plane and lean outward away from it, against its outward normal; `reversed =
true` leans them in.

See also: `doc.faces`, `design.thickness`

### One side of a block drafted 10 degrees from its bottom

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

## design.thickness

`face_point` and `face_normal` name the face it opens and are required; `faces
= {{point = .., normal = ..}, ...}` opens several, replacing that one.

`value` is the wall, 1 mm when left out, inside the solid; `inward = false`
puts the walls outside it, `both_sides = true` on both.

See also: `doc.faces`, `design.draft`

### A block hollowed into an open box

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

## design.delete_faces

`face_point` and `face_normal` name a face and are required; `faces = {{point
= .., normal = ..}, ...}` deletes several, replacing that one. A bore's wall
faces its axis: its normal points inward.

Faces whose removal leaves a neighbour with nothing around it (a blind hole's
wall without its bottom) fail at `pc.doc.rebuild()`; delete them together.

See also: `doc.faces`, `design.recognize_holes`

### A through hole closed by deleting its wall

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

## design.offset_faces

`face_point` and `face_normal` name a face and are required; `faces` lists
several, replacing that one.

`distance` (1 mm when left out) moves the faces along their outward normals. A
bore's outward normal points at its axis, so a positive distance makes a hole
smaller and a negative one wider.

See also: `design.move_faces`, `doc.faces`

### A hole widened by 1 mm all round

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

## design.scale_body

`factor` is 2 when left out and scales every axis alike; `per_axis = true`
takes `factors = {x, y, z}` instead. `factor = 25.4` makes a part modelled in
inches millimetres.

`about` is the point it scales about, in the body's own frame: "Origin" (the
default), "BodyCentre" (the middle of the solid's bounding box) or {Datum =
{datum = id}}, a datum's origin.

See also: `design.scaled`

### A box doubled about its centre, then stretched along X alone

```lua
local body = pc.doc.new_body{}
pc.design.primitive{body = body, kind = {Box = {length = 10, width = 10, height = 10}}}
local scale = pc.design.scale_body{body = body, factor = 2, about = "BodyCentre"}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
assert(math.abs(m.volume - 8000) < 1e-6)
assert(math.abs(m.min[1] + 5) < 1e-6 and math.abs(m.max[1] - 15) < 1e-6)
pc.design.set{feature = scale, per_axis = true, factors = {3, 1, 1}, about = "Origin"}
assert(#pc.doc.rebuild() == 0)
m = pc.doc.measure{body = body}
assert(math.abs(m.max[1] - 30) < 1e-6 and math.abs(m.max[2] - 10) < 1e-6)
```

## design.offset_solid

`distance` is 1 mm when left out: every face moves that far along its outward
normal, the faces around it following, so a box grows twice the distance each
way and a hole in it narrows. A negative distance shrinks the solid.

The kernel offsets planes, cylinders, cones, spheres and tori; a solid with a
free-form face, or one a shrink would collapse, fails on the feature with the
kernel's reason.

See also: `design.offset_faces`, `design.thickness`

### A box grown by a millimetre, then shrunk by one

```lua
local body = pc.doc.new_body{}
pc.design.primitive{body = body, kind = {Box = {length = 10, width = 10, height = 10}}}
local offset = pc.design.offset_solid{body = body, distance = 1}
assert(#pc.doc.rebuild() == 0)
local size = function()
  local m = pc.doc.measure{body = body}
  return m.max[1] - m.min[1], m.max[2] - m.min[2], m.max[3] - m.min[3]
end
local x, y, z = size()
assert(math.abs(x - 12) < 1e-6 and math.abs(y - 12) < 1e-6 and math.abs(z - 12) < 1e-6)
pc.design.set{feature = offset, distance = -1}
assert(#pc.doc.rebuild() == 0)
x = size()
assert(math.abs(x - 8) < 1e-6 and math.abs(pc.doc.measure{body = body}.volume - 512) < 1e-6)
```

## design.move_faces

`face_point` and `face_normal` name a face and are required; `faces` lists
several, replacing that one.

`translation` is 1 mm out along the face's normal when left out; give
`translation = {0, 0, 0}` for a turn alone. `angle_deg` turns the faces about
the line through `axis_point` (the face's point) along `axis_dir` (Z), in the
body's own frame.

See also: `design.offset_faces`, `doc.faces`

### A block's top face tilted about its middle

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

## design.project

`sketch` is required, and `face_point` with `face_normal` names a face to
project onto; `faces = {{point = .., normal = ..}, ...}` lists several,
replacing that one.

The sketch's curves are carried along its plane's normal, either way, to the
faces, landing on the side nearest the sketch: put the sketch beside the
faces, not through the solid.

`mode` is "Emboss" (the default: the regions the curves enclose stand out by
`height`), "Deboss" (they sink in by `height`) or "Split" (the faces are only
split along the curves). `height` is 1 mm when left out.

`wrap = true` wraps the sketch onto the curved surface the faces lie on
instead, its middle landing where the straight projection puts it. On a
cylinder or cone it is unrolled, every length kept, its width running round
the axis and its height along it, and the sketch's plane must not stand square
to the axis. On a sphere, a torus or a free-form face it is laid as a flat
sheet: lengths out from its middle are kept, lengths across them stretch with
the surface's curvature. Every face must lie on that one surface.

See also: `doc.faces`, `sketch.new`

### A raised label on a cylinder's side

```lua
local body = pc.doc.new_body{}
pc.design.primitive{body = body,
  kind = {Cylinder = {radius = 10, height = 20, angle_deg = 360}}}
-- A sketch standing in front of the side, facing away from it.
local label = pc.sketch.new{body = body, origin = {0, -15, 10},
  normal = {0, -1, 0}, x_axis = {1, 0, 0}}
pc.sketch.rect{sketch = label, x = -3, y = -2, width = 6, height = 4}
pc.design.project{sketch = label, mode = "Emboss", height = 1,
  face_point = {0, -10, 10}, face_normal = {0, -1, 0}}
assert(#pc.doc.rebuild() == 0, "the emboss builds")
local volume = pc.doc.measure{body = body}.volume
-- A ring sector 10 to 11 mm out, as wide as the label, 4 mm tall.
local added = 2 * math.asin(0.3) * (121 - 100) / 2 * 4
assert(math.abs(volume - (math.pi * 100 * 20 + added)) < 0.1 * added, volume)
```

### A band wrapped half way round a cylinder

```lua
local body = pc.doc.new_body{}
pc.design.primitive{body = body,
  kind = {Cylinder = {radius = 10, height = 20, angle_deg = 360}}}
local band = pc.sketch.new{body = body, origin = {0, -15, 10},
  normal = {0, -1, 0}, x_axis = {1, 0, 0}}
-- 30 mm long: wider than the cylinder, half way round it.
pc.sketch.rect{sketch = band, x = -15, y = -2, width = 30, height = 4}
pc.design.project{sketch = band, wrap = true, height = 1,
  face_point = {0, -10, 10}, face_normal = {0, -1, 0}}
assert(#pc.doc.rebuild() == 0, "the wrap builds")
local volume = pc.doc.measure{body = body}.volume
-- 30 x 4 x 1, a ring sector 10 to 11 mm out.
local added = 30 * 4 * 1 * (1 + 1 / 20)
assert(math.abs(volume - (math.pi * 100 * 20 + added)) < 0.01 * added, volume)
```

## design.text

`face_point` and `face_normal` name the face to write on, in the body's own
frame, and are required. The text stands in a sketch of its own, hidden under
the feature, 2 mm off the face and facing out, its middle where the face was
named: up along a cylinder's or cone's axis, else as near the body's +Z as the
face allows.

`text` ("Text"; a `{formula}` shows its value), `font`, `size` (5 mm),
`spacing`, `angle`, `across` and `up` (mm the middle moves on the face) set
the text; `mode` ("Emboss", "Deboss" or "Split") and `height` (0.6 mm) what it
does to the face.

On a curved face the text wraps onto it (`wrap`, set by itself when the face
is curved): round a cylinder or cone, or with `along = true` along its axis;
`full_turn = true` spreads the letters evenly round a whole cylinder (`radius`
says how far round that is). The face's kind is read from the body's mesh, so
on a body not built yet give `wrap = true` and `radius` yourself.

See also: `design.project`, `sketch.text`

### A raised label on a box's top

```lua
local body = pc.doc.new_body{}
pc.design.primitive{body = body, kind = {Box = {length = 40, width = 20, height = 10}}}
local label = pc.design.text{body = body, text = "OK", size = 8, height = 1,
  face_point = {20, 10, 10}, face_normal = {0, 0, 1}}
assert(#pc.doc.rebuild() == 0, "the text builds")
local m = pc.doc.measure{body = body}
assert(m.max[3] > 10.9 and m.max[3] < 11.1, "letters stand 1 mm proud")
assert(m.volume > 40 * 20 * 10, "material added")
```

### Letters sunk into a cylinder's side

```lua
local body = pc.doc.new_body{}
pc.design.primitive{body = body,
  kind = {Cylinder = {radius = 10, height = 20, angle_deg = 360}}}
pc.design.text{body = body, text = "ABC", size = 4, mode = "Deboss",
  height = 0.5, face_point = {0, -10, 10}, face_normal = {0, -1, 0}}
assert(#pc.doc.rebuild() == 0, "the text builds")
local volume = pc.doc.measure{body = body}.volume
assert(volume < math.pi * 100 * 20 - 1, "material cut away")
```

## design.mirror, design.linear_pattern, design.polar_pattern, design.circular_pattern, design.point_pattern, design.path_pattern, design.scaled

`originals = {id, ...}` are the features it repeats; left out, it takes the
body's last feature that is not a dress-up, a pattern, a mirror or a boolean.
It is refused in a body with no solid feature yet.

A copy that lands outside the solid (a pocket repeated past the material)
changes nothing, without an error.

`skipped = {1, 3}` leaves occurrences out by index: 0 is the original, then
each copy in turn (a mirror's copy is 1). Skipping 0 leaves the originals' own
material out, and is refused when an original is the body's first solid.

## design.mirror

The plane is YZ through the body's origin when left out; `plane = "XY"` or
`"XZ"` takes another, and `face_point` with `face_normal` a flat face of the
solid.

See also: `design.linear_pattern`

### A post mirrored across the YZ plane

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

## design.linear_pattern

`axis` is X when left out (Y, Z, or `{Custom = {origin = {x, y, z}, dir = {x,
y, z}}}`); `occurrences` (3) counts the original; `length` (30 mm) runs from
the first to the last, or is the gap between them with `spacing_mode = true`.

See also: `design.polar_pattern`

### A hole repeated three times along X

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 30, height = 10}
local pad = pc.design.pad{sketch = s, length = 4}
local body = pc.doc.feature{id = pad}.body
local at = pc.sketch.new{body = body, plane = "XY", offset = 4}
pc.sketch.circle{sketch = at, x = 5, y = 5, radius = 2}
local hole = pc.design.pocket{sketch = at, through_all = true}
-- Three holes 10 mm apart: 20 mm from the first to the last.
local pattern = pc.design.linear_pattern{body = body, originals = {hole}, axis = "X", length = 20, occurrences = 3}
assert(#pc.doc.rebuild() == 0, "the pattern builds")
local xs = {}
for _, face in ipairs(pc.doc.faces{body = body}) do
  if face.kind == "cylinder" then xs[#xs + 1] = face.axis.point[1] end
end
table.sort(xs)
assert(#xs == 3 and math.abs(xs[1] - 5) < 1e-6 and math.abs(xs[3] - 25) < 1e-6)
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (30 * 10 - 3 * math.pi * 4) * 4) < 1e-3, volume)
-- The middle hole left out: the original is occurrence 0.
pc.design.set{feature = pattern, skipped = {1}}
assert(#pc.doc.rebuild() == 0, "the pattern builds")
volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (30 * 10 - 2 * math.pi * 4) * 4) < 1e-3, volume)
```

## design.polar_pattern

`axis` is Z through the body's origin when left out; `occurrences` (4) counts
the original. `angle_deg` of 360 (the default) spreads them evenly round the
turn; less puts the first and the last at its two ends.

See also: `design.linear_pattern`

### Six holes round a disc

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

## design.circular_pattern

Rings about `axis` (Z through the body's origin when left out): ring i is
moved i × `spacing` (10 mm) along `outward` (X), then spread evenly round the
axis, `counts[i]` occurrences in it ({6, 12}). The original is the first of
the first ring, which turns it in place, so `outward` should run from the axis
through it.

See also: `design.polar_pattern`

### Two rings of holes round a disc, four and eight

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 30}
local disc = pc.design.pad{sketch = s, length = 3}
local body = pc.doc.feature{id = disc}.body
local at = pc.sketch.new{body = body, plane = "XY", offset = 3}
pc.sketch.circle{sketch = at, x = 10, y = 0, radius = 2}
local hole = pc.design.pocket{sketch = at, through_all = true}
-- The inner ring at radius 10, the outer 10 mm further out.
pc.design.circular_pattern{body = body, originals = {hole}, axis = "Z",
  outward = "X", spacing = 10, counts = {4, 8}}
assert(#pc.doc.rebuild() == 0, "the rings build")
local holes = 0
for _, face in ipairs(pc.doc.faces{body = body}) do
  if face.kind == "cylinder" and face.radius < 3 then holes = holes + 1 end
end
assert(holes == 12, holes)
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (math.pi * 900 - 12 * math.pi * 4) * 3) < 1e-3, volume)
```

## design.point_pattern

`sketch` gives the points: a copy stands at each of its points after the
first, moved by how far that point is from the first, so the original should
stand at the first point drawn. Construction points are left out; a curve's
ends and centres count as points.

See also: `design.hole`

### A hole at each corner of a plate

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 40, height = 30}
local plate = pc.design.pad{sketch = s, length = 4}
local body = pc.doc.feature{id = plate}.body
local at = pc.sketch.new{body = body, plane = "XY", offset = 4}
pc.sketch.circle{sketch = at, x = 5, y = 5, radius = 1.5}
local hole = pc.design.pocket{sketch = at, through_all = true}
-- The first point is where the hole stands.
local corners = pc.sketch.new{body = body, plane = "XY", offset = 4}
for _, p in ipairs({{5, 5}, {35, 5}, {35, 25}, {5, 25}}) do
  pc.sketch.point{sketch = corners, x = p[1], y = p[2]}
end
pc.design.point_pattern{sketch = corners, originals = {hole}}
assert(#pc.doc.rebuild() == 0, "the pattern builds")
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (1200 - 4 * math.pi * 2.25) * 4) < 1e-3, volume)
```

## design.path_pattern

`sketch` gives the path, its first chain of curves (`path = {Sketch = {sketch
= id, chain = n}}` another, counted from 0); `path = {Edge = {point = {x, y,
z}, direction = {x, y, z}}}` an edge of the solid near the point, in the
body's own frame, found on the solid as it stands where the pattern does.

`occurrences` (4) counts the original, spread over the whole path, evenly
round a closed one; `spacing_mode = true` places them `spacing` mm apart along
it instead. The original stands for the path's start (its end with `reversed =
true`): each copy is moved from there to its place, and turned as the path
turns with `align = true`, so round a circle an aligned copy goes round its
centre wherever the original is.

See also: `design.polar_pattern`, `doc.edges`

### Five holes along an arc

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = -40, y = -40, width = 80, height = 80}
local plate = pc.design.pad{sketch = s, length = 4}
local body = pc.doc.feature{id = plate}.body
local at = pc.sketch.new{body = body, plane = "XY", offset = 4}
pc.sketch.circle{sketch = at, x = 30, y = 0, radius = 2}
local hole = pc.design.pocket{sketch = at, through_all = true}
-- A quarter arc of radius 30 from where the hole stands.
local arc = pc.sketch.new{body = body, plane = "XY", offset = 4}
pc.sketch.arc{sketch = arc, x = 0, y = 0, radius = 30, start = 0, ["end"] = 90}
pc.design.path_pattern{sketch = arc, originals = {hole}, occurrences = 5}
assert(#pc.doc.rebuild() == 0, "the pattern builds")
local ys = {}
for _, face in ipairs(pc.doc.faces{body = body}) do
  if face.kind == "cylinder" then ys[#ys + 1] = face.axis.point[2] end
end
table.sort(ys)
assert(#ys == 5 and math.abs(ys[5] - 30) < 1e-6, #ys)
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (6400 - 5 * math.pi * 4) * 4) < 1e-3, volume)
```

## design.scaled

It makes a Multi Transform whose fields are `originals` and `steps`, one
`{Scale = {factor, center, occurrences}}`: 1.5 about the body's origin, 2
occurrences, when left out. The occurrences count the original and grow evenly
up to `factor`. A `factor` given on its own is refused.

`steps` may hold any of `{Linear = {axis, length, occurrences}}`, `{Polar =
{axis, angle_deg, occurrences}}`, `{Mirror = {plane}}`, `{Scale = ...}`,
`{Circular = {axis, outward, spacing, counts}}` (concentric rings about
`axis`, ring i moved i × `spacing` along `outward`, `counts[i]` round it) and
`{Points = {sketch}}` (a copy at each of the sketch's points after its first,
moved by how far it is from the first) and `{Path = {path, occurrences,
spacing_mode, spacing, reversed, align}}` (as `design.path_pattern` takes
them), each applied to every result of the steps before.

See also: `design.linear_pattern`

### A block repeated at 1.5 and 2 times its size

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

## design.boolean

`body` is the one it changes; `tool_body` (the latest other body when left
out) is what it takes, and keeps its own solid. It is refused while the
document has no other body.

`kind` is Fuse when left out, or Cut or Common; `more_tools = {id, ...}` takes
further bodies the same way.

See also: `doc.new_body`, `doc.set_visible`

### A corner cut out of a block by another body

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

## design.set

It builds nothing: `pc.doc.rebuild()` builds the change and tells what fails.
A misspelt field is refused, naming the fields the feature has.

A sketch is not a Design feature and is refused; the `sketch.*` commands
change it.

See also: `doc.feature`, `doc.set_formula`

### A pad made taller

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

## design.datum

A plane on a face (`mode = "face"`) has its origin at `face_point` and follows
the face when the solid changes; a sketch stands on the datum with
`pc.sketch.new{body = ..., on = datum}`.

`design.set{feature = datum, offset = {x, y, z}}` moves it later, and what
stands on it follows at the next `pc.doc.rebuild()`.

See also: `sketch.new`, `design.set`

### A boss on a plane that follows the top face

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

### A post standing on the plane midway between a block's top and bottom

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 8}
local body = pc.doc.feature{id = pad}.body
-- Halfway between the top and the bottom: z = 4, its origin
-- between the points the faces were given at.
local middle = pc.design.datum{kind = "plane", body = body, mode = "midplane",
  planes = {{face_point = {10, 5, 8}, face_normal = {0, 0, 1}},
            {face_point = {10, 5, 0}, face_normal = {0, 0, -1}}}}
local on = pc.sketch.new{body = body, on = middle}
pc.sketch.circle{sketch = on, x = 0, y = 0, radius = 2}
pc.design.pad{sketch = on, length = 10}
assert(#pc.doc.rebuild() == 0, "the post builds")
assert(math.abs(pc.doc.measure{body = body}.max[3] - 14) < 1e-6)
-- A taller block moves the middle, and the post, up by half.
pc.design.set{feature = pad, length = 12}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.max[3] - 16) < 1e-6)
```

## design.borrow

A feature takes a borrowed sketch by the borrow's id as its `sketch`. A pad or
a pocket takes a lent face with `profile_borrowed = {borrow = id, index = 0}`,
once the lending body is built (`pc.doc.rebuild()`); before, it is refused
asking for a sketch.

Lent faces serve elsewhere too: an up-to-shape pad or pocket stops on them
with `extras = {up_to_shape_borrowed = {{borrow = id, index = 0}}}`, a
revolution or groove turns until one with `mode = {UpToBorrowed = {borrow =
id, index = 0}}`, and `design.map_sketch` puts a sketch on one.

Borrowing a sketch of the same body is refused (a feature takes it directly),
as is giving both `sketch` and `from`.

See also: `design.freeze`, `design.map_sketch`, `doc.faces`

### A sketch and a face of one body padded in another

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
