# Sketcher external geometry commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## sketch.external_from

Every curve and loose point of the other sketch comes, its construction left
out; a datum comes as one element.

It only guides unless `counts = true`: a sketch holding guides alone has no
profile, and a pad of it fails at `pc.doc.rebuild()`.

It returns {elements, constraints}: the curves and the points at their ends.
Only the curves are external geometry, which `pc.sketch.geometry` marks
`external`.

`from` must be a sketch other than this one, or a datum; anything else is
refused.

The elements follow their source without this sketch being opened: when the
other sketch or the datum moves (by hand or by a formula),
`pc.sketch.geometry` reads them where it now is and the next
`pc.doc.rebuild()` builds from that. A source curve that becomes another kind
of curve, or is deleted, is caught up with when this sketch is next edited.

What the sketch already holds comes once: bringing the same sketch again adds
only what is new in it, and is refused ("... already in the sketch ...") when
nothing is.

See also: `sketch.external_defining`, `sketch.carbon_copy`

### A circle from the sketch below, counted, padded and following its source

```lua
local body = pc.doc.new_body{name = "Boss"}
local base = pc.sketch.new{body = body, plane = "XY"}
local circle = pc.sketch.circle{sketch = base, x = 0, y = 0, radius = 5}
local radius = pc.sketch.constrain{sketch = base, kind = "radius", items = {circle}, value = 5}
local top = pc.sketch.new{body = body, plane = "XY", offset = 10}
local made = pc.sketch.external_from{sketch = top, from = base, counts = true}
assert(#made.elements == 2, "the circle and its centre")
pc.design.pad{sketch = top, length = 3}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
assert(math.abs(m.volume - math.pi * 25 * 3) < 0.01, m.volume)
assert(math.abs(m.min[3] - 10) < 1e-6)
pc.sketch.set_value{sketch = base, constraint = radius[1], value = 6}
assert(#pc.doc.rebuild() == 0)
m = pc.doc.measure{body = body}
assert(math.abs(m.volume - math.pi * 36 * 3) < 0.01, "the copy follows: " .. m.volume)
```

## sketch.external_defining

Every item must be external geometry, or the call is refused ("... is not
external geometry"); the elements `pc.sketch.geometry` marks `external` are.
The points at an external curve's ends and centre go with their curve, so the
`elements` `sketch.external`, `sketch.external_from` and `sketch.intersection`
return can be passed as they come.

Counting takes an element out of construction; `on = false` makes it a guide
again.

See also: `sketch.external_from`, `sketch.external`

### A guide made to count

```lua
local body = pc.doc.new_body{name = "Boss"}
local base = pc.sketch.new{body = body, plane = "XY"}
pc.sketch.circle{sketch = base, x = 0, y = 0, radius = 5}
local top = pc.sketch.new{body = body, plane = "XY", offset = 10}
local made = pc.sketch.external_from{sketch = top, from = base}
local pad = pc.design.pad{sketch = top, length = 3}
assert(#pc.doc.rebuild() == 1, "a guide alone is no profile")
pc.sketch.external_defining{sketch = top, items = made.elements}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = body}
assert(math.abs(m.volume - math.pi * 25 * 3) < 0.01, m.volume)
```

## sketch.external

Each edge is a point on it and its direction there, in the body's own frame;
`pc.doc.edges` gives both (where the body sits, the same until it is moved).
The body must be built first (`pc.doc.rebuild()`), else it is refused ("that
body has no solid shape").

It only guides unless `counts = true`; counted, the projected edges close a
profile as drawn lines do. An edge square to the sketch plane projects to a
point.

It returns {elements, constraints}: the curves and the points at their ends,
the curves marked `external` by `pc.sketch.geometry`.

The projection follows its solid: rebuilt, the edge is found again by the
faces it runs between (else by the point) and projected anew, the sketch
unopened. An edge of the sketch's own body is taken from the solid as it
stands where the sketch is in the history, so what the sketch builds never
moves it; another body's solid built from the sketch is projected again only
when the sketch is opened.

A point names an edge only within a tenth of the body's diagonal of it; one
farther from every edge is refused ("no edge near ..."). An edge the sketch
already holds comes once: picked again, at any point along it, it is passed
over, and a call that brings nothing new is refused ("... already in the
sketch ...").

See also: `sketch.intersection`, `doc.edges`, `sketch.external_defining`

### A block's top edges projected and padded higher

```lua
local body = pc.doc.new_body{name = "Block"}
local base = pc.sketch.new{body = body, plane = "XY"}
pc.sketch.rect{sketch = base, x = 0, y = 0, width = 20, height = 10}
pc.design.pad{sketch = base, length = 5}
assert(#pc.doc.rebuild() == 0)
local top = pc.sketch.new{body = body, plane = "XY", offset = 5}
local made = pc.sketch.external{sketch = top, counts = true, edges = {
  {body = body, point = {10, 0, 5}, direction = {1, 0, 0}},
  {body = body, point = {20, 5, 5}, direction = {0, 1, 0}},
  {body = body, point = {10, 10, 5}, direction = {1, 0, 0}},
  {body = body, point = {0, 5, 5}, direction = {0, 1, 0}},
}}
assert(#made.elements > 0)
pc.design.pad{sketch = top, length = 3}
assert(#pc.doc.rebuild() == 0, "the four top edges close a profile")
assert(math.abs(pc.doc.measure{body = body}.volume - 20 * 10 * 8) < 1e-3)
```

## sketch.intersection

Each face is a point on it and its outward normal there, in the body's own
frame; `pc.doc.faces` gives both. The body must be built first.

A face the sketch plane does not cross adds nothing. What it adds only guides
unless `counts = true`.

It returns {elements, constraints}: the curves and the points at their ends.

See also: `sketch.external`, `doc.faces`

### Where a block's top face crosses a sketch through its middle

```lua
local body = pc.doc.new_body{name = "Block"}
local base = pc.sketch.new{body = body, plane = "XY"}
pc.sketch.rect{sketch = base, x = 0, y = 0, width = 20, height = 10}
pc.design.pad{sketch = base, length = 5}
assert(#pc.doc.rebuild() == 0)
local cut = pc.sketch.new{body = body, plane = "XZ", offset = -5}
local made = pc.sketch.intersection{sketch = cut, faces = {{body = body, point = {10, 5, 5}, normal = {0, 0, 1}}}}
assert(#made.elements == 3, "a line and its two ends")
for _, e in ipairs(pc.sketch.geometry{sketch = cut}) do
  if e.kind == "line" then
    assert(e.external and e.points[1][2] == 5 and e.points[2][2] == 5, "the top face, at sketch y = 5")
  end
end
```
