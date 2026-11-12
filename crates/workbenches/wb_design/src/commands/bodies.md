# Design body commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## design.recognize_holes

The body must be built first (`pc.doc.rebuild()`); a body with no round holes
gets nothing, `holes = 0`.

Per set of alike holes it adds one Delete Faces, a sketch of their centres and
a Hole, the Hole last; `design.set` on that Hole resizes them all.

See also: `design.hole`, `design.delete_faces`

### Two bores made a Hole feature and widened

```lua
local body = pc.doc.new_body{}
pc.design.primitive{body = body, kind = {Box = {length = 20, width = 10, height = 5}}}
local at = pc.sketch.new{body = body, plane = "XY", offset = 5}
pc.sketch.circle{sketch = at, x = 5, y = 5, radius = 2}
pc.sketch.circle{sketch = at, x = 15, y = 5, radius = 2}
pc.design.pocket{sketch = at, through_all = true}
assert(#pc.doc.rebuild() == 0)
local made = pc.design.recognize_holes{body = body}
-- The two alike bores: one Delete Faces, one sketch of centres and one Hole.
assert(made.holes == 2 and made.left == 0 and #made.features == 3)
assert(#pc.doc.rebuild() == 0, "the holes drill again")
local hole = made.features[3]
assert(pc.doc.feature{id = hole}.fields.Hole.diameter == 4)
pc.design.set{feature = hole, diameter = 6}
assert(#pc.doc.rebuild() == 0)
local volume = pc.doc.measure{body = body}.volume
assert(math.abs(volume - (1000 - 2 * math.pi * 9 * 5)) < 1e-3, volume)
```

## design.split

Given neither a plane nor a sketch, it cuts across the middle of the solid,
square to its longest side: a part too long for the bed in two.

Each piece starts with a Clone of the body split and a Split feature keeping
its side, so editing the body split rebuilds the pieces; the body split is
hidden. `design.set` of one piece's `plane`, `offset` or `sketch` moves the
cut of every piece, and its `other_side` swaps the sides the two keep.

The body must be built first (`pc.doc.rebuild()`); a mesh body is refused.

See also: `design.set`, `doc.measure`

### A plate split in two, and the pieces following an edit

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 40, height = 20}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
assert(#pc.doc.rebuild() == 0)
local pieces = pc.design.split{body = body, plane = "YZ", offset = 10}
assert(#pieces == 2 and #pc.doc.rebuild() == 0)
local volume = function(b) return pc.doc.measure{body = b}.volume end
assert(math.abs(volume(pieces[1]) - 3000) < 1e-3, volume(pieces[1]))
assert(math.abs(volume(pieces[2]) - 1000) < 1e-3, volume(pieces[2]))
-- The pieces follow the body split.
pc.design.set{feature = pad, length = 10}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(volume(pieces[1]) + volume(pieces[2]) - 8000) < 1e-3)
-- One piece's cut moved moves the other's.
local cut = pc.doc.features{body = pieces[1]}[2].id
pc.design.set{feature = cut, offset = 20}
assert(#pc.doc.rebuild() == 0)
assert(math.abs(volume(pieces[2]) - 4000) < 1e-3, volume(pieces[2]))
```

### A plate split by a sketch's circle: the disc and the plate around it

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 40, height = 20}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
local cut = pc.sketch.new{body = body, plane = "XY"}
pc.sketch.circle{sketch = cut, x = 20, y = 10, radius = 5}
assert(#pc.doc.rebuild() == 0)
local pieces = pc.design.split{body = body, sketch = cut}
assert(#pc.doc.rebuild() == 0)
local disc = math.pi * 25 * 5
assert(math.abs(pc.doc.measure{body = pieces[1]}.volume - disc) < 1e-3)
assert(math.abs(pc.doc.measure{body = pieces[2]}.volume - (4000 - disc)) < 1e-3)
```

## design.separate

Each piece starts with a Clone of the body and a Piece keeping the solid
nearest a point on that piece, so editing the body rebuilds the pieces; the
body itself is hidden. A solid of one piece is refused.

The body must be built first (`pc.doc.rebuild()`).

See also: `doc.measure`

### A plate pocketed through into two pieces, separated

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 40, height = 20}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
local slot = pc.sketch.new{body = body, plane = "XY", offset = 5}
pc.sketch.rect{sketch = slot, x = 10, y = -1, width = 2, height = 22}
pc.design.pocket{sketch = slot, through_all = true}
assert(#pc.doc.rebuild() == 0)
local pieces = pc.design.separate{body = body}
assert(#pieces == 2 and #pc.doc.rebuild() == 0)
local a = pc.doc.measure{body = pieces[1]}.volume
local b = pc.doc.measure{body = pieces[2]}.volume
assert(math.abs(a + b - 3800) < 1e-3, a + b)
assert(math.abs(math.min(a, b) - 1000) < 1e-3, math.min(a, b))
-- A body of one piece has nothing to separate.
assert(not pcall(pc.design.separate, {body = pieces[1]}))
```

## design.section

Each sketch holds the cut's outline on its plane: the outer boundary and every
hole, its curves exact (lines, arcs, circles, ellipses, splines) and their
ends joined, so a closed cut pads. They are taken once and do not follow the
solid.

The body must be built first (`pc.doc.rebuild()`). A plane that misses the
solid is refused; with several cuts, those that miss give empty sketches.

See also: `design.pad`, `sketch.regions`

### A plate with a hole cut across its middle and the outline padded again

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 40, height = 20}
pc.sketch.circle{sketch = s, x = 20, y = 10, radius = 5}
local pad = pc.design.pad{sketch = s, length = 6}
local body = pc.doc.feature{id = pad}.body
assert(#pc.doc.rebuild() == 0)
local cut = pc.design.section{body = body, plane = "XY", offset = 3}
assert(#cut == 1)
local again = pc.design.pad{sketch = cut[1], length = 2}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = again}.body}
assert(math.abs(m.volume - 2 * (800 - math.pi * 25)) < 1e-3, m.volume)
```

### Three cuts 2 mm apart, each a sketch

```lua
local body = pc.doc.new_body{}
pc.design.primitive{body = body, kind = {Box = {length = 10, width = 10, height = 10}}}
assert(#pc.doc.rebuild() == 0)
local cuts = pc.design.section{body = body, plane = "XY", offset = 2, count = 3, spacing = 2}
assert(#cuts == 3)
```
