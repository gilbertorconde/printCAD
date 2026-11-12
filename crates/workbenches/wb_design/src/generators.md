# Design generator commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## design.gear, design.sprocket, design.shaft, design.keyway

It makes only the sketch, whose curves its numbers fix: a pad or a revolution
built from it makes the solid, and `sketch.generator` changes the numbers
afterwards.

Without `body` it goes in the selected body, else in a new one: in a script
each call without `body` starts a body of its own.

A field the generator lacks is refused, naming the ones it has; fields left
out keep their defaults, which `pc.doc.feature{id = ...}.fields.generator`
shows.

See also: `sketch.generator`

## design.gear

`module` is in mm (2 when left out) and `teeth` 20: the pitch diameter is
module times teeth, the tip diameter two modules more. It lies on XY centred
on the origin, with a 5 mm `bore` (0 for none).

A bore that does not fit inside the root circle is refused, and no body is
made for it.

A gear of few teeth (below about 17 at 20°) is undercut at its root as a rack
cutter leaves it; `pc.sketch.generator` answers `undercut = true` for it.
`keyway = {on = true}` cuts the standard key's keyway into the bore.

See also: `design.pad`

### A 12-tooth gear padded 5 mm

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

## design.sprocket

`pitch` and `roller` are the chain's, in mm (12.7 and 8.51 when left out),
with 18 `teeth` and an 8 mm `bore`. It lies on XY centred on the origin.

See also: `design.pad`

### A 9-tooth sprocket, its diameters read back

```lua
local sprocket = pc.design.sprocket{teeth = 9, bore = 6}
local size = pc.sketch.generator{sketch = sprocket}
-- A chain's pitch circle: the pitch over the sine of half a tooth's angle.
assert(math.abs(size.pitch_diameter - 12.7 / math.sin(math.pi / 9)) < 1e-3)
local pad = pc.design.pad{sketch = sprocket, length = 3}
assert(#pc.doc.rebuild() == 0, "the sprocket builds")
```

## design.keyway

The slot is `length` long end to end and as wide as the key, centred on the
plane's origin or on `face_point`, running along x or `along`.
`pc.sketch.generator{sketch = id}` answers its `width` and the `depth` to
pocket it; a shaft outside 6 to 230 mm has no standard key and needs both
given.

On a shaft turned about Z with `design.shaft`, give the face as a point on its
round side and the outward normal there, and `along = {0, 0, 1}`.

See also: `design.pocket`, `design.shaft`

### A keyway pocketed into a 20 mm shaft

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

## design.shaft

It draws the half section on XZ, its sections from z = 0 upward, each a
`length` and a `diameter` in mm, with a `chamfer` and a `fillet`;
`start_chamfer` (0.5 mm when left out) breaks the bottom edge.
`design.revolve` with no axis turns it about Z into the shaft.

See also: `design.revolve`

### A two-step shaft turned from its section

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
