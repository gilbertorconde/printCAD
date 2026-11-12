# Assembly joint commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

In an example, a line `{{box}}` or `{{facing}}` stands for the setup the
code hands this file under that name.

## asm.mate

`offset` is the gap between the faces in mm, 0 when left out; `flip = true`
turns the body so both faces point the same way.

It holds the faces together and nothing else: the body keeps a turn about the
normal and two slides along the face, so it is not centred and stays where it
was across the face.

Both faces must be flat: a round face is refused as having no normal.

See also: `asm.distance`, `asm.flip`

### A lid set on a base

```lua
{{box}}
{{facing}}
local base = box(0, 20, 20, 5)
local lid = box(40, 10, 10, 3)
assert(#pc.doc.rebuild() == 0)
pc.asm.mate{body = lid, face = facing(lid, -1),
  other = base, other_face = facing(base, 1)}
local at = pc.asm.placement{body = lid}.translation
assert(at[1] == 0 and at[2] == 0 and math.abs(at[3] - 5) < 1e-4,
  "lifted onto the top, not moved across")
assert(pc.asm.freedom{body = lid}[1].free == 3,
  "it may still slide and turn on the face")
```

## asm.align

The two axes go on one line; the body keeps a turn about it and a slide along
it.

`turn_drive` (degrees from where it was made) and `slide_drive` (mm) hold
those motions; `turn_limits` and `slide_limits` keep them in a range; false
takes each away.

`asm.travel` refuses an alignment: it reads a hinge or a slider, which have
one motion.

See also: `asm.hinge`, `asm.slider`

### A wheel on a shaft, held 8 mm up

```lua
local function pin(x, r, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.circle{sketch = s, x = x, y = 0, radius = r}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local function round(body)
  for _, f in ipairs(pc.doc.faces{body = body}) do
    if f.axis then return f end
  end
end
local shaft = pin(0, 5, 20)
local wheel = pin(40, 15, 4)
assert(#pc.doc.rebuild() == 0)
pc.asm.align{body = wheel, face = round(wheel),
  other = shaft, other_face = round(shaft), slide_drive = 8}
local at = pc.asm.placement{body = wheel}.translation
assert(math.abs(at[1] + 40) < 1e-3 and math.abs(at[3] - 8) < 1e-3,
  "on the shaft's axis, 8 mm up")
assert(pc.asm.freedom{body = wheel}[1].free == 1,
  "the slide is held, the turn is free")
```

## asm.angle

`degrees` is between the outward normals, or the axes; left out, the angle
they make now is kept.

It holds only the angle: the body may still slide every way and turn about the
other axes, five motions left.

See also: `asm.parallel`, `asm.perpendicular`

### A plate held at 30 degrees to a base

```lua
{{box}}
{{facing}}
local base = box(0, 20, 20, 5)
local plate = box(40, 10, 10, 2)
assert(#pc.doc.rebuild() == 0)
pc.asm.angle{body = plate, face = facing(plate, 1),
  other = base, other_face = facing(base, 1), degrees = 30}
local w = pc.asm.placement{body = plate}.rotation[4]
assert(math.abs(math.deg(2 * math.acos(w)) - 30) < 1e-3, "turned 30 degrees")
assert(pc.asm.freedom{body = plate}[1].free == 5, "only the angle is held")
```

## asm.hinge

The axes go on one line and the body keeps one motion, the turn about it.
`offset` is how far along the axis the first sits from the second, mm.

`drive` holds the angle in degrees from where the hinge was made, positive
turning right-handed about the axis's direction; false lets it turn again.

`limits = {low, high}` keeps the angle in that range while it is not driven; a
body outside it is brought to the nearer end.

See also: `asm.travel`, `asm.turn`, `asm.couple`, `asm.motion`

### An arm on a post, turned a quarter turn

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
local a = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = a, x = -2, y = 20, width = 30, height = 4}
local arm = pc.doc.feature{id = pc.design.pad{sketch = a, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
local axis = {axis = {point = {0, 22, 0}, direction = {0, 0, 1}}}
local post_axis = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
local h = pc.asm.hinge{body = arm, face = axis,
  other = post, other_face = post_axis}
assert(pc.asm.freedom{body = arm}[1].free == 1, "it turns about the post")
pc.asm.set{joint = h, drive = 90}
assert(math.abs(pc.asm.travel{joint = h} - 90) < 1e-3)
local q = pc.asm.placement{body = arm}.rotation
assert(math.abs(q[3] - math.sin(math.rad(45))) < 1e-4, "a quarter turn about +Z")
```

## asm.slider

The axes go on one line and the body keeps one motion, the slide along it; it
does not turn.

`drive` holds the position in mm from where the slider was made, positive
along the axis's direction; `limits` keeps it within {low, high}.

See also: `asm.travel`, `asm.couple`, `asm.align`

### A carriage on a rail, 30 mm along

```lua
{{box}}
local rail = box(0, 100, 10, 5)
local carriage = box(0, 20, 10, 5)
assert(#pc.doc.rebuild() == 0)
local below = {axis = {point = {0, 5, 0}, direction = {1, 0, 0}}}
local along = {axis = {point = {0, 5, 5}, direction = {1, 0, 0}}}
local s = pc.asm.slider{body = carriage, face = below,
  other = rail, other_face = along, limits = {0, 80}}
pc.asm.set{joint = s, drive = 30}
local at = pc.asm.placement{body = carriage}.translation
assert(math.abs(at[1] - 30) < 1e-3 and math.abs(at[3] - 5) < 1e-3,
  "on the rail, 30 mm along")
assert(math.abs(pc.asm.travel{joint = s} - 30) < 1e-3)
```

## asm.fix

With no faces it holds the body to the other where both sit now: nothing moves
as it is made, and no motion is left.

The fixed body follows the other at the next solve: `asm.move` and `asm.place`
of the other do not carry it until then.

See also: `asm.group`, `asm.ground`

### A tag that follows its base

```lua
{{box}}
local base = box(0, 20, 20, 5)
local tag = box(30, 5, 5, 5)
assert(#pc.doc.rebuild() == 0)
pc.asm.fix{body = tag, other = base}
assert(pc.asm.freedom{body = tag}[1].free == 0)
pc.asm.move{body = base, by = {0, 0, 10}}
pc.asm.solve{}
local at = pc.asm.placement{body = tag}.translation
assert(math.abs(at[3] - 10) < 1e-4, "the tag follows the base")
```

## asm.parallel

It turns the body until the faces or axes are parallel and holds only that:
the body may still slide every way and turn about the normal, four motions
left.

See also: `asm.angle`, `asm.perpendicular`, `asm.mate`

### A tilted plate turned back square

```lua
{{box}}
local base = box(0, 20, 20, 5)
local plate = box(40, 10, 10, 2)
assert(#pc.doc.rebuild() == 0)
pc.asm.move{body = plate, turn = 20, axis = {0, 1, 0}}
local top = pc.doc.faces{body = plate}[6]
local base_top = pc.doc.faces{body = base}[6]
pc.asm.parallel{body = plate, face = top, other = base, other_face = base_top}
local w = pc.asm.placement{body = plate}.rotation[4]
assert(math.abs(w - 1) < 1e-6, "turned back square")
assert(pc.asm.freedom{body = plate}[1].free == 4,
  "it still slides every way and turns about Z")
```

## asm.perpendicular

It turns the body until the faces or axes are square to each other and holds
only that, five motions left.

See also: `asm.parallel`, `asm.angle`

### A fin stood square to a base

```lua
{{box}}
local base = box(0, 20, 20, 5)
local fin = box(40, 10, 10, 2)
assert(#pc.doc.rebuild() == 0)
pc.asm.move{body = fin, turn = 60, axis = {0, 1, 0}}
pc.asm.perpendicular{body = fin, face = pc.doc.faces{body = fin}[6],
  other = base, other_face = pc.doc.faces{body = base}[6]}
local n = pc.doc.faces{body = fin}[6].normal
assert(math.abs(n[3]) < 1e-4, "the fin's face stands square to the base's top")
assert(pc.asm.freedom{body = fin}[1].free == 5)
```

## asm.distance

Between flat faces, `offset` is measured along the second face's normal, mm;
left out, the distance they are apart now is kept.

It holds only that distance: unlike a mate with an offset, the body may still
tilt and slide, five motions left.

Faces, axes `{axis}` and points `{centre}` or `{point}` mix: a face's distance
from a point, an axis's from an axis.

See also: `asm.mate`

### A plate held 10 mm above a base

```lua
{{box}}
{{facing}}
local base = box(0, 20, 20, 5)
local plate = box(40, 10, 10, 2)
assert(#pc.doc.rebuild() == 0)
pc.asm.distance{body = plate, face = facing(plate, -1),
  other = base, other_face = facing(base, 1), offset = 10}
local at = pc.asm.placement{body = plate}.translation
assert(math.abs(at[3] - 15) < 1e-4, "10 mm above the base's top")
```

## asm.tangent

One face is flat and the other round, either way round; two of one kind are
refused.

The round face's radius comes from the face, as `pc.doc.faces` lists it with
`radius`, or from the `radius` argument; with neither the joint is refused.

It leaves the body four motions.

See also: `asm.mate`, `asm.cam`

### A roller against a block's side

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 40, height = 40}
local base = pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
local r = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = r, x = 80, y = 0, radius = 3}
local roller = pc.doc.feature{id = pc.design.pad{sketch = r, length = 10}}.body
assert(#pc.doc.rebuild() == 0)
local side, round
for _, f in ipairs(pc.doc.faces{body = base}) do
  if f.normal and f.normal[1] > 0.99 then side = f end
end
for _, f in ipairs(pc.doc.faces{body = roller}) do
  if f.axis then round = f end
end
pc.asm.tangent{body = roller, face = round, other = base, other_face = side}
local at = pc.asm.placement{body = roller}.translation
assert(math.abs(80 + at[1] - 43) < 1e-3, "its axis 3 mm out from x = 40")
```

## asm.ball

It takes points: a ball's `{centre}`, else `{point}`; a flat face given whole
is taken at its listed point.

The two points meet and the body may turn every way, three motions left.

See also: `asm.universal`, `asm.distance`

### An arm's corner on a base's corner

```lua
{{box}}
local base = box(0, 40, 40, 5)
local arm = box(60, 30, 4, 4)
assert(#pc.doc.rebuild() == 0)
pc.asm.ball{body = arm, face = {point = {60, 0, 4}},
  other = base, other_face = {point = {40, 40, 5}}}
local at = pc.asm.placement{body = arm}.translation
assert(math.abs(at[1] + 20) < 1e-4 and math.abs(at[2] - 40) < 1e-4
  and math.abs(at[3] - 1) < 1e-4, "the arm's corner on the base's corner")
assert(pc.asm.freedom{body = arm}[1].free == 3, "it turns every way")
```

## asm.universal

Each body gives a pin, `{axis = {point, direction}}`. The two axes' points
meet and the body keeps two turns, one about each pin.

See also: `asm.ball`, `asm.hinge`

### Two pins crossed at one point

```lua
{{box}}
local base = box(0, 40, 40, 5)
local arm = box(60, 30, 4, 4)
assert(#pc.doc.rebuild() == 0)
local along_x = {axis = {point = {60, 2, 2}, direction = {1, 0, 0}}}
local up_z = {axis = {point = {20, 20, 10}, direction = {0, 0, 1}}}
pc.asm.universal{body = arm, face = along_x, other = base, other_face = up_z}
local free = pc.asm.freedom{body = arm}[1]
assert(free.free == 2 and free.motions[1].turn and free.motions[2].turn)
local at = pc.asm.placement{body = arm}.translation
assert(math.abs(at[1] + 40) < 1e-3 and math.abs(at[3] - 8) < 1e-3,
  "the two pins cross at (20, 20, 10)")
```

## asm.slot

`face` is the pin, a point (`{centre}` or `{point}`) on the moving body;
`other_face` the slot, a line `{axis = {point, direction}}` on the other.

The pin stays on the line: the body keeps three turns and the slide along it.

See also: `asm.path`, `asm.slider`

### A pin kept on a line

```lua
{{box}}
local base = box(0, 40, 40, 5)
local arm = box(60, 30, 4, 4)
assert(#pc.doc.rebuild() == 0)
local edge = {axis = {point = {0, 40, 5}, direction = {1, 0, 0}}}
local pin = {point = {60, 0, 0}}
pc.asm.slot{body = arm, face = pin, other = base, other_face = edge}
local at = pc.asm.placement{body = arm}.translation
assert(math.abs(at[2] - 40) < 1e-4 and math.abs(at[3] - 5) < 1e-4,
  "the pin on the base's back top edge")
local free = pc.asm.freedom{body = arm}[1]
assert(free.free == 4 and free.motions[4].slide[1] == 1,
  "three turns and a slide along X")
```

## asm.path

`other_face` is only a `{point}` near the edge: the other body's edge nearest
it is taken, however far, and kept with the joint.

The moving point goes onto that edge where the edge is nearest to it, not to
the point given.

See also: `asm.slot`, `asm.cam`

### A corner run along a disc's rim

```lua
local d = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = d, x = 0, y = 0, radius = 20}
local disc = pc.doc.feature{id = pc.design.pad{sketch = d, length = 5}}.body
local r = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = r, x = 60, y = 0, width = 4, height = 4}
local rider = pc.doc.feature{id = pc.design.pad{sketch = r, length = 4}}.body
assert(#pc.doc.rebuild() == 0)
pc.asm.path{body = rider, face = {point = {60, 0, 0}},
  other = disc, other_face = {point = {0, 20, 5}}}
local at = pc.asm.placement{body = rider}.translation
local x, y, z = 60 + at[1], at[2], at[3]
assert(math.abs(math.sqrt(x * x + y * y) - 20) < 0.01 and math.abs(z - 5) < 1e-3,
  "the rider's corner on the disc's top rim")
```

## asm.cam

`other_face` is a `{point}` near the cam's face: the other body's face nearest
it is taken.

`radius` is the roller's, mm: the follower's point keeps that far off the
face; 0 when left out, a point follower.

See also: `asm.path`, `asm.tangent`

### A follower 2 mm off a round cam

```lua
local d = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = d, x = 0, y = 0, radius = 20}
local cam = pc.doc.feature{id = pc.design.pad{sketch = d, length = 5}}.body
local r = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = r, x = 60, y = 0, width = 4, height = 4}
local follower = pc.doc.feature{id = pc.design.pad{sketch = r, length = 4}}.body
assert(#pc.doc.rebuild() == 0)
pc.asm.cam{body = follower, face = {point = {60, 0, 0}},
  other = cam, other_face = {point = {20, 0, 2}}, radius = 2}
local p = pc.asm.placement{body = follower}
local q, t = p.rotation, p.translation
-- Where the follower's point (60, 0, 0) is now: turned by q, then moved by t.
local x = 60 * (1 - 2 * (q[2] ^ 2 + q[3] ^ 2)) + t[1]
local y = 60 * 2 * (q[1] * q[2] + q[3] * q[4]) + t[2]
assert(math.abs(math.sqrt(x * x + y * y) - 22) < 0.1,
  "2 mm off the cam's 20 mm face")
```

## asm.width

It takes four flat faces: `face` and `face2`, the tab's two sides, and
`other_face` and `other_face2`, the slot's two walls.

The tab is centred between the walls and keeps three motions: two slides along
the walls and a turn about their normal.

See also: `asm.mate`, `asm.distance`

### A tab centred between two walls

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 5, height = 20}
pc.sketch.rect{sketch = s, x = 15, y = 0, width = 5, height = 20}
local walls = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
local t = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = t, x = 40, y = 0, width = 6, height = 10}
local tab = pc.doc.feature{id = pc.design.pad{sketch = t, length = 10}}.body
assert(#pc.doc.rebuild() == 0)
local function side(body, sign, x)
  for _, f in ipairs(pc.doc.faces{body = body}) do
    local n = f.normal
    if n and n[1] * sign > 0.99 and math.abs(f.point[1] - x) < 1e-3 then
      return f
    end
  end
end
pc.asm.width{body = tab, face = side(tab, -1, 40), face2 = side(tab, 1, 46),
  other = walls, other_face = side(walls, 1, 5), other_face2 = side(walls, -1, 15)}
local at = pc.asm.placement{body = tab}.translation
assert(math.abs(at[1] + 33) < 1e-3, "the 6 mm tab centred in the 10 mm gap")
```

## asm.couple

Both joints must be hinges or sliders. `gearing` left out is the first that
suits them: gears for two hinges, a rack and pinion for a hinge and a slider.

`ratio` is 1 when left out for gears and a belt, a 10 mm pitch radius for a
rack and a 2 mm lead for a screw; it must be above zero.

The tie starts where both joints stand as it is made. Moving either moves the
other: `asm.turn` on the driven hinge turns the driver too.

See also: `asm.hinge`, `asm.slider`, `asm.set`

### Two gears, the second half as fast the other way

```lua
local function disc(x, r)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.circle{sketch = s, x = x, y = 30, radius = r}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = 3}}.body
end
local f = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = f, x = -50, y = 0, width = 100, height = 10}
local frame = pc.doc.feature{id = pc.design.pad{sketch = f, length = 2}}.body
local small, big = disc(0, 10), disc(30, 20)
assert(#pc.doc.rebuild() == 0)
local function at(x)
  return {axis = {point = {x, 30, 0}, direction = {0, 0, 1}}}
end
local h1 = pc.asm.hinge{body = small, face = at(0), other = frame, other_face = at(0)}
local h2 = pc.asm.hinge{body = big, face = at(30), other = frame, other_face = at(30)}
pc.asm.couple{driver = h1, driven = h2, ratio = 0.5}
pc.asm.set{joint = h1, drive = 40}
assert(math.abs(pc.asm.travel{joint = h2} + 20) < 1e-3,
  "gears: half as far, the other way")
```

## asm.set

Only what is given changes; a setting the joint's kind does not have, such as
`degrees` on a mate, is passed over without an error.

`kind` makes the joint again as that kind from where the bodies stand, keeping
its faces and its name; the new kind's settings start afresh (a mate at offset
0) unless given in the same call. A kind that does not take the faces is
refused.

`face`, `other` and `other_face` pick again, in world space as `pc.doc.faces`
lists them. `moving_end` and `fixed_end` move each end of the joint along its
own normal or axis, mm.

Given a coupling, it changes `gearing`, `ratio`, `reverse`, `driver` and
`driven`; a gearing that does not suit the joints is refused.

See also: `asm.couple`, `asm.turn`

### A mate's gap changed, then the mate made a distance

```lua
{{box}}
{{facing}}
local base = box(0, 20, 20, 5)
local lid = box(40, 10, 10, 3)
assert(#pc.doc.rebuild() == 0)
local j = pc.asm.mate{body = lid, face = facing(lid, -1),
  other = base, other_face = facing(base, 1)}
local function height() return pc.asm.placement{body = lid}.translation[3] end
pc.asm.set{joint = j, offset = 2}
assert(math.abs(height() - 7) < 1e-4, "a 2 mm gap")
pc.asm.set{joint = j, kind = "distance", offset = 4}
assert(pc.doc.feature{id = j}.kind == "Distance")
assert(math.abs(height() - 9) < 1e-4, "4 mm apart")
```

## asm.redundant

A joint is listed when its body's other joints already hold all it holds, such
as a parallel beside a mate of the same faces. Nothing is removed.

See also: `asm.freedom`

### A parallel that a mate makes needless

```lua
{{box}}
{{facing}}
local base = box(0, 20, 20, 5)
local lid = box(40, 10, 10, 3)
assert(#pc.doc.rebuild() == 0)
pc.asm.mate{body = lid, face = facing(lid, -1),
  other = base, other_face = facing(base, 1)}
local p = pc.asm.parallel{body = lid, face = facing(lid, 1),
  other = base, other_face = facing(base, 1)}
local extra = pc.asm.redundant{}
assert(#extra == 1 and extra[1].joint == p and extra[1].name == "Parallel 1",
  "the mate already keeps them parallel")
```

## asm.select_joints

The joints the body moves by and those that hold another body to it, a ground
on it too, suppressed ones as well. The tree selects them together, the first
the active one; with none, the document is selected.

See also: `asm.redundant`, `asm.unmet`

### A lid mated to a grounded base, and a block mated onto the lid

```lua
{{box}}
{{facing}}
local base = box(0, 20, 20, 5)
local lid = box(40, 10, 10, 3)
local block = box(80, 4, 4, 3)
assert(#pc.doc.rebuild() == 0)
local g = pc.asm.ground{body = base}
local m = pc.asm.mate{body = lid, face = facing(lid, -1),
  other = base, other_face = facing(base, 1)}
pc.asm.mate{body = block, face = facing(block, -1),
  other = lid, other_face = facing(lid, 1)}
local held = pc.asm.select_joints{body = base}
assert(#held == 2 and held[1].joint == g and held[2].joint == m,
  "the ground and the mate onto it")
assert(#pc.asm.select_joints{body = lid} == 2, "the lid's mate and the one onto it")
```

## asm.unmet

What a solve left apart: joints asking more of their bodies than they can give
at once, or a coupling its joints cannot follow. An assembly that holds lists
nothing.

See also: `asm.redundant`, `asm.select_joints`

### A mate that holds

```lua
{{box}}
{{facing}}
local base = box(0, 20, 20, 5)
local lid = box(40, 10, 10, 3)
assert(#pc.doc.rebuild() == 0)
pc.asm.mate{body = lid, face = facing(lid, -1),
  other = base, other_face = facing(base, 1)}
assert(#pc.asm.unmet{} == 0, "the mate holds")
```

## asm.travel

It is read from where the bodies sit now, counted from where the joint was
made, so a body moved by hand reads its new travel before any solve.

Any other joint is refused, an alignment too.

See also: `asm.hinge`, `asm.slider`, `asm.turn`

### A hinge's angle after its arm is moved

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
local a = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = a, x = -2, y = -2, width = 30, height = 4}
local arm = pc.doc.feature{id = pc.design.pad{sketch = a, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
local h = pc.asm.hinge{body = arm, face = z, other = post, other_face = z}
assert(pc.asm.travel{joint = h} == 0, "0 where it was made")
pc.asm.move{body = arm, turn = 45}
assert(math.abs(pc.asm.travel{joint = h} - 45) < 1e-3,
  "read from where the arm sits")
```

## asm.ground

Grounding a body already grounded answers its ground joint again. The
document's first joint grounds the body its `other` names, so a ground is
often there already.

See also: `asm.fix`, `asm.freedom`

### A base kept where it is

```lua
{{box}}
{{facing}}
local base = box(0, 20, 20, 5)
local lid = box(40, 10, 10, 3)
assert(#pc.doc.rebuild() == 0)
local g = pc.asm.ground{body = base}
assert(pc.asm.ground{body = base} == g, "grounding twice keeps one ground")
pc.asm.mate{body = lid, face = facing(lid, -1),
  other = base, other_face = facing(base, 1)}
assert(pc.asm.placement{body = base}.translation[3] == 0, "the ground stays")
assert(pc.asm.ground{body = base, grounded = false} == nil)
```

## asm.freedom

Grounded bodies, and bodies no joint moves, are not listed. A held drive takes
its motion away; a limit does not: a hinge, slider or alignment resting on a
limit keeps the motion, `at_limit`. A point on a path keeps its run along it
at a corner too, and at an open path's end `at_limit`.

`through` is a point on a turn's axis, in world space.

See also: `asm.redundant`, `asm.solve`

### What a mate leaves a lid

```lua
{{box}}
{{facing}}
local base = box(0, 20, 20, 5)
local lid = box(40, 10, 10, 3)
assert(#pc.doc.rebuild() == 0)
pc.asm.mate{body = lid, face = facing(lid, -1),
  other = base, other_face = facing(base, 1)}
local all = pc.asm.freedom{}
assert(#all == 1 and all[1].body == lid, "the grounded base is not listed")
local turns, slides = 0, 0
for _, m in ipairs(all[1].motions) do
  if m.turn then turns = turns + 1; assert(m.turn.axis[3] == 1) end
  if m.slide then slides = slides + 1; assert(m.slide[3] == 0) end
end
assert(turns == 1 and slides == 2,
  "on a face: a turn about its normal and two slides along it")
```

## asm.solve

The joint commands, `asm.set` and `asm.ground` solve as they run; `asm.place`
and `asm.move` do not, so solve after them.

It answers "Moved 1 body; every joint holds", or "Every joint holds" when
nothing had to move.

A joint moves the body it belongs to; one that belongs to a grounded body
moves the body at its other end instead. A joint between two bodies that
cannot move (both grounded) and does not hold is refused, named ("cannot hold
all its joints at once").

See also: `asm.place`, `asm.move`

### A lid put back on its base

```lua
{{box}}
{{facing}}
local base = box(0, 20, 20, 5)
local lid = box(40, 10, 10, 3)
assert(#pc.doc.rebuild() == 0)
pc.asm.mate{body = lid, face = facing(lid, -1),
  other = base, other_face = facing(base, 1)}
local function height() return pc.asm.placement{body = lid}.translation[3] end
pc.asm.place{body = lid, translation = {0, 0, 30}}
assert(height() == 30, "a placement does not solve")
assert(pc.asm.solve{} == "Moved 1 body; every joint holds")
assert(math.abs(height() - 5) < 1e-4)
assert(pc.asm.solve{} == "Every joint holds")
```
