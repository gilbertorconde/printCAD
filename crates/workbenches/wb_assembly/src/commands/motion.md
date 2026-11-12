# Assembly motion commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

In an example, a line `{{box}}` stands for the setup the code hands this
file under that name.

## asm.motion

Each formula gives its drive's value at time `t`, seconds: a hinge's angle in
degrees, a slider's position in mm. A plain number takes the drive's unit; a
formula that does not read, or gives a length for a hinge ("1 in * t") or an
angle for a slider, is refused, and so is a joint that is not a hinge or a
slider.

`end` is a Lua keyword: write `["end"] = 1`. `start`, `end` and `step` are 0,
2 and 0.05 s when left out.

Making it moves nothing: `asm.motion_frames` and `asm.trace` play it on a
copy. `study` changes a motion already made, keeping its id.

See also: `asm.motion_frames`, `asm.trace`, `asm.motion_clashes`

### A hinge turned 90 degrees a second, then changed

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
local m = pc.asm.motion{drives = {{joint = h, formula = "90 * t"}}, ["end"] = 1}
assert(#pc.asm.motion_frames{study = m} == 21, "0 to 1 s every 0.05 s")
local slower = {{joint = h, formula = "45 * t"}}
pc.asm.motion{study = m, drives = slower, ["end"] = 1, step = 0.5}
assert(#pc.asm.motion_frames{study = m} == 3, "the same motion, changed")
assert(pc.asm.travel{joint = h} == 0, "making it moves nothing")
```

## asm.motion_frames

Frames run from `start` to `end`, both included, every `step`: 0 to 1 s every
0.25 s is 5 frames. Every body is in every frame.

Each frame solves the joints with every drive held at its formula's value, on
a copy: the bodies stay where they are. An id that is not a motion is refused
("is not a motion").

See also: `asm.motion`, `asm.trace`

### Where an arm is at the end of its motion

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
local drives = {{joint = h, formula = "90 * t"}}
local m = pc.asm.motion{drives = drives, ["end"] = 1, step = 0.25}
local frames = pc.asm.motion_frames{study = m}
local last = frames[#frames]
assert(#frames == 5 and last.t == 1)
for _, b in ipairs(last.bodies) do
  if b.body == arm then
    local turned = math.deg(2 * math.acos(b.rotation[4]))
    assert(math.abs(turned - 90) < 1e-3, "a quarter turn at 1 s")
  end
end
assert(pc.asm.placement{body = arm}.rotation[4] == 1, "the arm has not moved")
```

## asm.trace

`point` is given in the body's own frame; each frame's `point` is where it is
in the world then. Nothing moves. An id that is not a motion is refused ("is
not a motion").

See also: `asm.motion_frames`, `asm.motion`

### The tip of a turning arm

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
local m = pc.asm.motion{drives = {{joint = h, formula = "90 * t"}}, ["end"] = 1}
local path = pc.asm.trace{study = m, body = arm, point = {28, 0, 0}}
local last = path[#path]
assert(math.abs(last.point[2] - 28) < 1e-3, "the arm's tip ends on +Y")
assert(math.abs(last.speed - 28 * math.pi / 2) < 0.1,
  "a quarter turn a second at 28 mm")
```

## asm.exploded_view

Each step moves its bodies on from the steps before it: first turned by
`turn`, then pushed out by `radial`, then moved by `shift`, mm in world space.
A step needs both `bodies` and `shift` ({0, 0, 0} for none); `turn` and
`radial` may be left out.

`turn = {point = {x, y, z}, axis = {x, y, z}, degrees = a}` turns the bodies
about the line through `point` along `axis`. `radial = {centre = {x, y, z},
distance = d}` pushes each body `d` mm straight away from `centre`, measured
from the middle of its box; with `axis = {x, y, z}` it pushes square away from
the line through `centre` along it.

Making it moves nothing: `asm.explode_at` answers where it puts the bodies.
`view` changes a view already made, keeping its id.

See also: `asm.explode_at`

### A lid lifted, then the base moved aside

```lua
{{box}}
local base = box(0, 20, 20, 5)
local lid = box(40, 10, 10, 3)
assert(#pc.doc.rebuild() == 0)
local view = pc.asm.exploded_view{steps = {
  {bodies = {lid}, shift = {0, 0, 20}},
  {bodies = {base}, shift = {-15, 0, 0}},
}}
for _, p in ipairs(pc.asm.explode_at{view = view, at = 2}) do
  if p.body == lid then assert(p.translation[3] == 20) end
  if p.body == base then assert(p.translation[1] == -15) end
end
assert(pc.asm.placement{body = lid}.translation[3] == 0, "nothing has moved")
```

### Two blocks spread apart from the point between them

```lua
local function box(x)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = 10, height = 10}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = 4}}.body
end
local left, right = box(0), box(30)
assert(#pc.doc.rebuild() == 0)
local view = pc.asm.exploded_view{steps = {
  {bodies = {left, right}, shift = {0, 0, 0},
   radial = {centre = {20, 5, 2}, distance = 10}},
}}
for _, p in ipairs(pc.asm.explode_at{view = view, at = 1}) do
  if p.body == left then assert(math.abs(p.translation[1] + 10) < 1e-3) end
  if p.body == right then assert(math.abs(p.translation[1] - 10) < 1e-3) end
end
```

## asm.explode_at

Past the last step it stays at the end. Every body of the document is listed,
moved by the view or not.

See also: `asm.exploded_view`

### Half way through the second step

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
local lid = pc.doc.feature{id = pc.design.pad{sketch = s, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
local view = pc.asm.exploded_view{steps = {
  {bodies = {lid}, shift = {0, 0, 20}},
  {bodies = {lid}, shift = {10, 0, 0}},
}}
local at = pc.asm.explode_at{view = view, at = 1.5}[1].translation
assert(at[1] == 5 and at[3] == 20, "the first step done, half the second")
local past = pc.asm.explode_at{view = view, at = 9}[1].translation
assert(past[1] == 10, "past the last step is the last step")
```

## asm.save_state

It keeps every body's placement, which bodies are hidden and the value each
drive holds. `state` saves the assembly as it is now over a state already
made, keeping its id.

See also: `asm.restore_state`

### A state saved again after a change

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
local a = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = a, x = -2, y = -2, width = 30, height = 4}
local arm = pc.doc.feature{id = pc.design.pad{sketch = a, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
local h = pc.asm.hinge{body = arm, face = z, other = post, other_face = z,
  drive = 0}
local closed = pc.asm.save_state{name = "Closed"}
pc.asm.set{joint = h, drive = 90}
pc.asm.save_state{state = closed}
pc.asm.set{joint = h, drive = 10}
pc.asm.restore_state{state = closed}
assert(math.abs(pc.asm.travel{joint = h} - 90) < 1e-3, "saved again at 90")
```

## asm.restore_state

It puts back the placements, the hidden bodies (showing the rest) and the
drives' values, then solves. Anything but a saved state is refused.

See also: `asm.save_state`

### An opened arm put back, shown again

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
local post = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
local a = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = a, x = -2, y = -2, width = 30, height = 4}
local arm = pc.doc.feature{id = pc.design.pad{sketch = a, length = 3}}.body
assert(#pc.doc.rebuild() == 0)
local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
local h = pc.asm.hinge{body = arm, face = z, other = post, other_face = z,
  drive = 0}
local closed = pc.asm.save_state{name = "Closed"}
pc.asm.set{joint = h, drive = 90}
pc.doc.set_visible{id = arm, visible = false}
pc.asm.restore_state{state = closed}
assert(math.abs(pc.asm.travel{joint = h}) < 1e-3, "the drive is back at 0")
for _, b in ipairs(pc.doc.bodies()) do
  if b.id == arm then assert(b.visible, "and the arm is shown again") end
end
```

## asm.motion_clashes

`steps` positions are checked, `low` and `high` among them: 3 from 0 to 180
are 0, 90 and 180. Each is solved on a copy: nothing moves.

A pair is listed only where it shares more material than it does where the
joint stands now, so a contact already there does not count. A joint that is
not a hinge or a slider is refused.

See also: `asm.interference`, `asm.motion`

### An arm swung into a post

```lua
local function box(x, y, w, h, len)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = y, width = w, height = h}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = len}}.body
end
local base = box(-5, -5, 10, 10, 2)
local arm = box(-2, -2, 30, 4, 3)
local post = box(0, 15, 4, 4, 10)
assert(#pc.doc.rebuild() == 0)
local z = {axis = {point = {0, 0, 0}, direction = {0, 0, 1}}}
local h = pc.asm.hinge{body = arm, face = z, other = base, other_face = z}
local clashes = pc.asm.motion_clashes{joint = h, low = 0, high = 180, steps = 3}
assert(#clashes == 1 and clashes[1].at == 90,
  "the arm hits the post a quarter turn round")
assert(clashes[1].volume > 0)
assert(pc.asm.travel{joint = h} == 0, "the arm is left where it was")
```
