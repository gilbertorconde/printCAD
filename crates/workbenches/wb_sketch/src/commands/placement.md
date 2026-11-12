# Sketcher sketch placement commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## sketch.new

Without `body` the sketch goes in the selected body, else in a new one: in a
script two sketches made without `body` land in two bodies, and a pocket or a
hole from the second is refused for want of material. Give `body` from
`pc.doc.feature{id = ...}.body` or `pc.doc.new_body`.

`body` takes any body, a surface body too: that is how a sketch starts in a
surface body, as the Surface bench's Create sketch does.

XY faces +Z, YZ faces +X and XZ faces -Y, so a pad from an XZ sketch grows
toward -Y and `offset` moves an XZ sketch toward -Y. The sketch's x and y run
along the plane's two letters (on XZ, y is world Z). Lower case names are
taken too.

With `generator` it is named after it (Sprocket, Sprocket_1, ...) and
`sketch.generator` sets its numbers, as `design.sprocket` and its kin do on a
base plane or a picked face.

See also: `doc.new_body`, `sketch.rect`, `design.datum`

### Two sketches in one body

```lua
local body = pc.doc.new_body{name = "Bracket"}
local base = pc.sketch.new{body = body, plane = "XY"}
local side = pc.sketch.new{body = body, plane = "XZ", offset = 5}
assert(pc.doc.feature{id = base}.body == body)
assert(pc.doc.feature{id = side}.body == body)
assert(#pc.doc.bodies() == 1, "both sketches went in the one body")
```

## sketch.attachment

A sketch on a datum moves on the datum and follows it; any other moves from
the plane, face or attachment it was placed on, and a face or attachment still
takes it along. Placing the sketch again (`sketch.set_plane`) drops the
offset.

Each value given replaces the one the sketch had rather than adding to it:
`offset = 5` twice leaves it 5 mm off.

`shift` runs along the plane's own x and y; `turn` is degrees about its
normal.

See also: `sketch.new`, `design.datum`, `sketch.set_plane`

### A sketch set 5 mm off its datum

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

## sketch.set_plane

The geometry keeps its sketch coordinates and moves with the plane. Without
`x_axis` the sketch's x is a direction square to the normal chosen for it (+Y
for a normal along X), so give `x_axis` to know which way the geometry lies.

The plane given is fixed: a sketch made on a datum stops following it, and
`sketch.attachment` refuses it from then on.

See also: `sketch.attachment`, `sketch.new`

### A sketch moved onto a plane facing +X

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

## sketch.mirror_sketch

It makes a new sketch, named after this one with "mirror", in the same body;
this one is left as it is. The mirror is across the sketch's own Y axis: x
becomes -x.

For both halves in one profile, `sketch.merge` the two, or mirror within one
sketch with `sketch.draw`'s mirror tool.

See also: `sketch.merge`, `sketch.draw`

### A rectangle mirrored across the Y axis

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

## sketch.merge

It makes a new sketch, named after this one with "merged", on this one's plane
and in its body; the sketches merged are left as they are. Constraints come
with the geometry; under a turn or a mirror those stated against the sketch
axes follow it (swapped on a quarter turn, made parallel or perpendicular to
one another at any other angle). Projected geometry stays projected from its
solid, sketch or datum.

Each sketch in `with` must lie on a plane parallel to this one ("its plane is
not parallel to this one"); its geometry is laid onto this plane, the distance
between them dropped.

See also: `sketch.carbon_copy`, `sketch.mirror_sketch`

### Two sketches merged and padded as one

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

## sketch.carbon_copy

The geometry comes with its constraints, as geometry of this sketch: free to
edit, not tied to the sketch it came from. Under a turn or a mirror the
constraints stated against the sketch axes follow it, and projected geometry
stays projected from its solid, sketch or datum.

`from` must lie on a plane parallel to this one ("its plane is not parallel to
this one"); a sketch cannot copy itself.

See also: `sketch.merge`, `sketch.external_from`

### A rectangle copied onto a sketch above it

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
