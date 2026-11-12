# Sketcher constraint commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## sketch.constrain

Points are elements of their own: a line's ends are the point ids at its
`start` and `end` in `pc.doc.feature{id = s}.fields.sketch.geometry`, the
line's own id is the line. A point made with `sketch.point` before the line is
the same id as the end drawn on it.

Without `value` a dimension takes what it measures on the sketch as it stands.
Angles are degrees, a diameter the diameter, a radius the radius.

"distance" on one line is its length (listed as Length), on two points the
distance between them, on a point and a curve or two curves the gap.
"dimension" picks as the toolbar does: a line's length, a circle's diameter,
an arc's radius, two lines' angle (their distance when parallel).

Nothing stays put until constrained: a dimension on free geometry moves every
item it names, so tie a corner to "origin" first to keep it where it was
drawn.

"lock" holds a point by its distances along X and Y from the origin, each the
size of a coordinate (a point at x = -5 takes 5 and stays at -5).

"group" makes its items one rigid unit, keeping the shape they have now: it
moves and turns as a whole, three degrees of freedom, and a drag of any member
carries all of it. Constraints among the members alone that say nothing of the
axes rest while the group holds; of those that do (horizontal, vertical, an
angle to an axis), the first turns the whole group and the rest rest.

A constraint that contradicts others is still added; `pc.sketch.status` names
the conflict. While it stands the sketch does not solve, gives no profile, and
what is built from it fails at `pc.doc.rebuild()` ("the sketch does not solve:
its constraints conflict"). `remove_redundant` takes away only older
constraints the new one repeats, never one it contradicts.

A dimension the sketch already holds is refused ("the sketch already holds
this dimension"), in any form that measures the same: a radius beside the
circle's diameter, a line's length beside a distance between its ends, a
distance from A to B beside one from B to A. A reference dimension or one
switched off holds its size too; `sketch.set_value` changes it.

A kind that does not fit the items is refused ("the ... constraint does not
fit these items"), one that is no kind at all is refused with the list of
kinds, and so is a `value` for a kind that takes none.

See also: `sketch.status`, `sketch.set_value`, `sketch.constraints`

### A plate fully constrained from its corner on the origin

```lua
local s = pc.sketch.new{plane = "XY"}
local corner = pc.sketch.point{sketch = s, x = 1, y = 1}
local sides = pc.sketch.rect{sketch = s, x = 1, y = 1, width = 20, height = 10}
pc.sketch.constrain{sketch = s, kind = "coincident", items = {corner, "origin"}}
pc.sketch.constrain{sketch = s, kind = "distance", items = {sides[1]}, value = 40}
pc.sketch.constrain{sketch = s, kind = "distance", items = {sides[2]}, value = 25}
assert(pc.sketch.status{sketch = s}.dof == 0, "fully constrained")
local pad = pc.design.pad{sketch = s, length = 2}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.volume - 40 * 25 * 2) < 1e-3)
assert(math.abs(m.min[1]) < 1e-6 and math.abs(m.min[2]) < 1e-6, "its corner on the origin")
```

### A rectangle grouped and dragged as one

```lua
local s = pc.sketch.new{plane = "XY"}
local sides = pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
pc.sketch.constrain{sketch = s, kind = "group", items = sides}
local status = pc.sketch.status{sketch = s}
assert(status.dof == 2, "square to the axes, free to slide")
assert(#status.redundant == 0, "its sides' alignments rest in the group")
pc.sketch.drag{sketch = s, items = {sides[1]}, by = {5, 3}}
local pad = pc.design.pad{sketch = s, length = 1}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.volume - 20 * 10) < 1e-2, "the same rectangle")
assert(math.abs(m.min[1] - 5) < 1e-4 and math.abs(m.min[2] - 3) < 1e-4, "moved whole")
```

## sketch.set_value

`constraint` is an id `sketch.constrain` returned; an element's id, or a
constraint that is not a dimension, is refused.

Angles are degrees; a diameter takes the diameter, a radius the radius.

To bind a dimension to a formula, `doc.set_formula` takes it by the key
`doc.parameters` lists for the sketch, which is the constraint's id.

See also: `sketch.constrain`, `doc.parameters`, `doc.set_formula`

### A circle's diameter changed after it was dimensioned

```lua
local s = pc.sketch.new{plane = "XY"}
local c = pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 5}
local d = pc.sketch.constrain{sketch = s, kind = "diameter", items = {c}}
pc.sketch.set_value{sketch = s, constraint = d[1], value = 20}
assert(pc.sketch.constraints{sketch = s}[1].value == 20)
local pad = pc.design.pad{sketch = s, length = 1}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.volume - math.pi * 10 ^ 2) < 0.01, m.volume)
```

## sketch.set_constraint

`items` are constraint ids; an element's id among them is refused. Only the
flags given change, and `driving = false` is refused for a constraint that is
not a dimension.

A dimension that is not driving measures and conflicts with nothing; a
constraint that is not active is kept but left out of solving.

`as` turns a radius into a diameter or back, keeping the constraint and the
size it holds: its value doubles or halves, as does what a formula on it
gives.

See also: `sketch.set_value`, `sketch.status`

### A repeated dimension made a reference

```lua
local s = pc.sketch.new{plane = "XY"}
local sides = pc.sketch.rect{sketch = s, x = 0, y = 0, width = 30, height = 15}
pc.sketch.constrain{sketch = s, kind = "distance", items = {sides[1]}, value = 30}
local top = pc.sketch.constrain{sketch = s, kind = "distance", items = {sides[3]}}
assert(#pc.sketch.status{sketch = s}.redundant > 0, "the top's length says the bottom's again")
pc.sketch.set_constraint{sketch = s, items = top, driving = false}
assert(#pc.sketch.status{sketch = s}.redundant == 0, "a reference only measures")
```

## sketch.remove_axis_alignment

It returns how many horizontal and vertical constraints went; one fewer
parallel or perpendicular constraint takes their place, so the group gains the
freedom to turn. Lines with none answer 0.

See also: `sketch.constrain`, `sketch.status`

### A rectangle freed to turn

```lua
local s = pc.sketch.new{plane = "XY"}
local sides = pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
assert(pc.sketch.status{sketch = s}.dof == 4)
assert(pc.sketch.remove_axis_alignment{sketch = s, items = sides} == 4)
for _, c in ipairs(pc.sketch.constraints{sketch = s}) do
  assert(c.kind == "Parallel" or c.kind == "Perpendicular", c.kind)
end
assert(pc.sketch.status{sketch = s}.dof == 5, "still a rectangle, free to turn")
```
