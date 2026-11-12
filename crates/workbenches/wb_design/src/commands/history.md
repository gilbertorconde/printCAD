# Design history commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## design.freeze

Any feature other than a borrow is refused.

See also: `design.borrow`

### A borrowed sketch frozen, then followed again

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local body = pc.doc.new_body{}
local lent = pc.design.borrow{body = body, sketch = s}
pc.design.pad{sketch = lent, length = 3}
pc.design.freeze{feature = lent}
-- A rectangle added to the source after the freeze is not taken.
pc.sketch.rect{sketch = s, x = 30, y = 0, width = 10, height = 10}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.volume - 600) < 1e-3)
pc.design.freeze{feature = lent, frozen = false}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.volume - 900) < 1e-3, "following it again")
```

## design.map_sketch

The sketch keeps its geometry in its own coordinates and moves with the plane;
a datum, mode or face it stood on before is left.

A borrow of another body, a curved face, or a face the lender has not built
yet (`pc.doc.rebuild()` first) is refused.

See also: `design.borrow`, `sketch.set_plane`

### A post's sketch moved onto a face another body lends

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 20}
local block = pc.design.pad{sketch = s, length = 10}
local lender = pc.doc.feature{id = block}.body
assert(#pc.doc.rebuild() == 0)
local body = pc.doc.new_body{}
local top = pc.design.borrow{body = body, from = lender,
  faces = {{point = {10, 10, 10}, normal = {0, 0, 1}}}}
local post = pc.sketch.new{body = body, plane = "XY"}
pc.sketch.circle{sketch = post, x = 10, y = 10, radius = 3}
pc.design.pad{sketch = post, length = 5}
pc.design.map_sketch{sketch = post, borrow = top}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.min[3] - 10) < 1e-6, "on the lent top")
-- The lender grows: the post rises with its top.
pc.design.set{feature = block, length = 15}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.min[3] - 15) < 1e-6, "follows the top")
```

## design.move_to_body

It is refused when the feature reads a sketch other features read too;
`design.duplicate` with `body` copies it there with a sketch of its own.

See also: `design.duplicate`, `doc.new_body`

### A boss moved into a body of its own

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
local top = pc.sketch.new{body = body, plane = "XY", offset = 5}
pc.sketch.rect{sketch = top, x = 0, y = 0, width = 5, height = 5}
local boss = pc.design.pad{sketch = top, length = 5}
local other = pc.doc.new_body{}
local moved = pc.design.move_to_body{feature = boss, body = other}
-- The boss went with the sketch only it used.
assert(#moved == 2 and moved[1] == top and moved[2] == boss)
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.volume - 1000) < 1e-3)
assert(math.abs(pc.doc.measure{body = other}.volume - 125) < 1e-3)
```

## design.duplicate

The copy is made exactly where the feature is, so in the same body it adds
nothing to the solid until `design.set` changes it.

See also: `design.set`, `design.move_to_body`

### A pad copied and turned to grow the other way

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
local made = pc.design.duplicate{feature = pad}
-- A sketch of its own and the copied pad, last.
assert(#made == 2 and made[1] ~= s)
local copy = made[2]
pc.design.set{feature = copy, reversed = true}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
assert(math.abs(m.volume - 1000) < 1e-3 and math.abs(m.min[3] + 5) < 1e-6, m.volume)
```
