# Assembly placing commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

In an example, a line `{{box}}` or `{{facing}}` stands for the setup the
code hands this file under that name.

## asm.turn

On a hinge the angle moves on by `degrees`: a drive holding it moves with it,
else the hinge is free again afterwards, where it was turned to. A hinge a
coupling drives turns its driver to get there.

Other joints turn the body about their normal or axis through the joint's
point, the joint carried with it so it holds the body there. A ground is
refused.

See also: `asm.flip`, `asm.travel`

### A mated block turned a quarter turn on its face

```lua
{{box}}
{{facing}}
local base = box(0, 20, 20, 5)
local lid = box(5, 10, 4, 3)
assert(#pc.doc.rebuild() == 0)
local j = pc.asm.mate{body = lid, face = facing(lid, -1),
  other = base, other_face = facing(base, 1)}
pc.asm.turn{joint = j, degrees = 90}
local q = pc.asm.placement{body = lid}.rotation
assert(math.abs(math.deg(2 * math.acos(q[4])) - 90) < 1e-3, "a quarter turn")
assert(math.abs(q[3]) > 0.7, "about Z")
assert(math.abs(facing(lid, -1).point[3] - 5) < 1e-4, "still on the top")
```

## asm.flip

The joint is carried with the body, so it holds it turned over. On a mate that
is `flip` in `asm.set`: a body resting on a face ends on its far side, inside
the body it rested on.

See also: `asm.turn`, `asm.set`

### A wheel turned over on its hinge

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
local w = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = w, x = 30, y = 0, radius = 10}
local wheel = pc.doc.feature{id = pc.design.pad{sketch = w, length = 4}}.body
assert(#pc.doc.rebuild() == 0)
local function axis(x)
  return {axis = {point = {x, 0, 0}, direction = {0, 0, 1}}}
end
local h = pc.asm.hinge{body = wheel, face = axis(30),
  other = post, other_face = axis(0), offset = 3}
pc.asm.flip{joint = h}
local q = pc.asm.placement{body = wheel}.rotation
assert(math.abs(q[4]) < 1e-4, "half a turn")
assert(math.abs(q[3]) < 1e-4, "about an axis square to the hinge's")
```

## asm.turn90

The turn the joint keeps is turned 90° about the other end's axis or normal: a
hinge's or an alignment's zero, a slider's or a fixed joint's held turn. The
body turns with it, and what the joint reads stays as it was: a hinge held at
30° is held at 30° from its new zero, its limits turned round with it.

A joint that keeps no turn of its own is refused. A mate's turn in its plane
is free, so `asm.turn` turns a mated body; it is the same for the other
joints.

See also: `asm.turn`, `asm.travel`

### A hinge held at 30° whose zero is turned a quarter turn

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
local w = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = w, x = 0, y = -2, width = 20, height = 4}
local arm = pc.doc.feature{id = pc.design.pad{sketch = w, length = 2}}.body
assert(#pc.doc.rebuild() == 0)
local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
local h = pc.asm.hinge{body = arm, face = z, other = post, other_face = z,
  drive = 30}
pc.asm.turn90{joint = h}
assert(math.abs(pc.asm.travel{joint = h} - 30) < 1e-3, "still 30° on the hinge")
local q = pc.asm.placement{body = arm}.rotation
assert(math.abs(math.deg(2 * math.acos(q[4])) - 120) < 1e-2,
  "the arm a quarter turn further round")
```

## asm.placement

`translation` is in mm, in world space: where the body's own origin is, turned
by `rotation`.

See also: `asm.place`, `asm.move`

### A body's placement before and after a move

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
local body = pc.doc.feature{id = pc.design.pad{sketch = s, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
local p = pc.asm.placement{body = body}
assert(p.translation[1] == 0 and p.rotation[4] == 1, "where it was modelled")
pc.asm.move{body = body, by = {5, 0, 0}, turn = 90}
p = pc.asm.placement{body = body}
assert(p.translation[1] == 5 and math.abs(p.rotation[3] - math.sqrt(0.5)) < 1e-6)
```

## asm.place

It sets the placement outright; what is left out keeps its value. The
quaternion is normalised, and a zero one is refused.

It does not solve: joints catch up at the next `asm.solve`. The other bodies
of a rigid component move with it.

A drag that carries a geared hinge past a whole turn records the couplings'
new counts in `turns`, so a replay counts as the drag did.

See also: `asm.move`, `asm.placement`, `asm.solve`

### A body lifted and turned

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
local body = pc.doc.feature{id = pc.design.pad{sketch = s, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
pc.asm.place{body = body, translation = {0, 0, 20}, rotation = {0, 0, 1, 1}}
local p = pc.asm.placement{body = body}
assert(p.translation[3] == 20)
local half = math.sqrt(0.5)
assert(math.abs(p.rotation[3] - half) < 1e-6, "the quaternion is normalised")
pc.asm.place{body = body, translation = {1, 2, 3}}
p = pc.asm.placement{body = body}
assert(math.abs(p.rotation[3] - half) < 1e-6, "the rotation left out is kept")
```

## asm.move

It turns first, `turn` degrees about `axis` through `about`, then steps by
`by`, mm in world space; both add to where the body is.

It does not solve: joints catch up at the next `asm.solve`. The other bodies
of a rigid component move with it.

See also: `asm.place`, `asm.solve`

### A plate turned about its own centre

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 20}
local body = pc.doc.feature{id = pc.design.pad{sketch = s, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
pc.asm.move{body = body, turn = 90, about = {10, 10, 0}}
local t = pc.asm.placement{body = body}.translation
assert(math.abs(t[1] - 20) < 1e-4 and math.abs(t[2]) < 1e-4,
  "turned in place about its centre")
pc.asm.move{body = body, by = {0, 0, 5}}
t = pc.asm.placement{body = body}.translation
assert(math.abs(t[3] - 5) < 1e-4, "steps add up")
```
