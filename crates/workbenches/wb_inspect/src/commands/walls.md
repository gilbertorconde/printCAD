# Inspection wall and fit commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## inspect.walls

Read inward from every vertex of the body's mesh, along its normal; a solid is
read from its exact surfaces. Surfaces within 60° of edge-on to a ray are
passed over. The map paints thin walls hot while the Inspection bench is
active.

## inspect.element_solid

A cylinder or cone covers the length its points cover along its axis, a plane
becomes a slab over the patch its points cover, sunk under it (against its
normal), and a sphere or torus is whole. Hand `kind` and `placement` to
pc.design.primitive in a new body to model from it; the Element as body tool
does both. A circle or line has no solid.

See also: `design.primitive`

### A scanned cylinder fitted and made a body again

```lua
local drum = pc.doc.new_body{}
pc.design.primitive{body = drum, variant = "cylinder",
  kind = {Cylinder = {radius = 5, height = 10, angle_deg = 360}}}
assert(#pc.doc.rebuild() == 0)
local path = os.tmpname() .. ".stl"
pc.file.export{path = path, bodies = {drum}}
pc.doc.set_visible{id = drum, visible = false}
pc.file.import{path = path, scan = true}
pc.doc.rebuild()
os.remove(path)
local scan
for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
local fit = pc.inspect.grow{kind = "cylinder", scan = scan, at = {5, 0, 5}}
local s = pc.inspect.element_solid{element = fit.element}
local copy = pc.doc.new_body{name = s.name}
pc.design.primitive{body = copy, kind = s.kind, placement = s.placement}
assert(#pc.doc.rebuild() == 0)
local v = pc.doc.measure{body = copy}.volume
assert(math.abs(v - math.pi * 25 * 10) < 0.02 * math.pi * 25 * 10, v)
```

## inspect.clearance

The two are measured as they sit now: place the part in the housing first (an
alignment, the Assembly's joints or a placement). Each of the part's points
reads its distance to the housing's surface, positive across a gap and
negative inside the housing's material. `interfering` is the share of measured
points inside it and `deepest` how far, `narrowest` the smallest gap when
nothing interferes, and `too_tight` the share with less room than `min_gap`.
The map paints interference red while the Inspection bench is active.

### A 10 mm cube in a 12 mm square pocket has 1 mm all round; moved 1.5 mm, it presses 0.5 mm into one wall.

```lua
local box = pc.doc.new_body{}
pc.design.primitive{body = box, kind = {Box = {length = 30, width = 30, height = 10}}}
pc.design.subtractive_primitive{body = box, kind = {Box = {length = 12, width = 12, height = 20}},
placement = {origin = {9, 9, 2}, x_axis = {1, 0, 0}, z_axis = {0, 0, 1}}}
local part = pc.doc.new_body{}
pc.design.primitive{body = part, kind = {Box = {length = 10, width = 10, height = 10}}}
assert(#pc.doc.rebuild() == 0)
pc.doc.set_body{body = part, translation = {10, 10, 4}}
local fit = pc.inspect.clearance{part = part, housing = box, min_gap = 0.5}
assert(fit.interfering == 0 and math.abs(fit.narrowest - 1) < 1e-6, fit.narrowest)
pc.doc.set_body{body = part, translation = {11.5, 10, 4}}
local tight = pc.inspect.clearance{part = part, housing = box}
assert(tight.interfering > 0 and math.abs(tight.deepest - 0.5) < 1e-6, tight.deepest)
```

## inspect.overhang

Each point of the mesh facing down reads how far it leans out past vertical: 0
a wall, 90 a ceiling. Surface facing up, and the face the body stands on at
its lowest point, read unmeasured. `share` is the part of the body's area past
the limit, which needs support; `steepest` the worst overhang, in degrees. The
map paints while the Inspection bench is active.

### A box standing on the bed needs no support; on its edge, half of it does.

```lua
local body = pc.doc.new_body{}
pc.design.primitive{body = body}
assert(#pc.doc.rebuild() == 0)
local o = pc.inspect.overhang{body = body}
assert(o.share == 0 and o.limit == 45, o.share)
local q = math.pi / 8
pc.doc.set_body{body = body, rotation = {x = math.sin(q), y = 0, z = 0, w = math.cos(q)}}
local tilted = pc.inspect.overhang{body = body, limit = 40}
assert(math.abs(tilted.steepest - 45) < 1e-3, tilted.steepest)
```
