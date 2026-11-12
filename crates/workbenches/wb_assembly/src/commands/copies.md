# Assembly copy commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

In an example, a line `{{box}}` or `{{facing}}` stands for the setup the
code hands this file under that name.

## asm.copy

A copy has no features of its own: it takes the source's shape and follows its
every rebuild, placed by its own placement.

Left out, `step` puts each copy beside the one before along X, the body's
width and a tenth apart (22 mm for a 20 mm body). `count` is held between 1
and 500.

With `around`, a whole turn (360, the default) is shared with the source, so 3
copies stand at 90, 180 and 270 degrees; a part turn puts the last copy at
`angle`. `around` needs a `direction`.

See also: `asm.mirror`, `asm.parts`

### Two copies in a row and a ring of three

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local pad = pc.design.pad{sketch = s, length = 5}
local body = pc.doc.feature{id = pad}.body
assert(#pc.doc.rebuild() == 0)
local copies = pc.asm.copy{body = body, count = 2}
assert(#copies == 2)
local second = pc.asm.placement{body = copies[2]}.translation
assert(math.abs(second[1] - 44) < 1e-4, "22 mm apart along X")
pc.doc.set_value{id = pad, parameter = "length", value = 8}
assert(#pc.doc.rebuild() == 0)
local volume = pc.doc.measure{body = copies[1]}.volume
assert(math.abs(volume - 20 * 10 * 8) < 1e-3, "the copy follows")
local z = {point = {0, 0, 0}, direction = {0, 0, 1}}
local ring = pc.asm.copy{body = body, count = 3, around = z}
local w = pc.asm.placement{body = ring[1]}.rotation[4]
assert(math.abs(math.deg(2 * math.acos(w)) - 90) < 1e-3,
  "a whole turn shared by four")
```

## asm.mirror

The plane is in world space. The copy keeps the identity placement and draws
the source mirrored, following its changes.

The mirrored mesh shows at once; its solid, which measuring and STEP export
read, is made by `pc.doc.rebuild()`.

A mirror of a mirrored copy is refused, as is a zero normal.

See also: `asm.copy`

### A block mirrored across X = 0

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 10, y = 0, width = 20, height = 10}
local body = pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
assert(#pc.doc.rebuild() == 0)
local m = pc.asm.mirror{body = body, point = {0, 0, 0}, normal = {1, 0, 0}}
local faces = pc.doc.faces{body = m}
assert(#faces == 6)
for _, f in ipairs(faces) do
  local x = f.point[1]
  assert(x <= -10 + 1e-4 and x >= -30 - 1e-4, "on the other side of X = 0")
end
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = m}.volume - 1000) < 1e-6, "its solid, mirrored")
```

## asm.replace

The replaced body is hidden, not deleted. Its joints move to the new body,
found again on its faces, and the new body is placed where they put it.

`kept` and `unmatched` list joint names. A body replacing itself is refused.

See also: `asm.copy`

### A lid swapped for a thicker one

```lua
{{box}}
{{facing}}
local base = box(0, 20, 20, 5)
local lid = box(40, 10, 10, 3)
local thicker = box(80, 12, 12, 4)
assert(#pc.doc.rebuild() == 0)
local j = pc.asm.mate{body = lid, face = facing(lid, -1),
  other = base, other_face = facing(base, 1)}
local report = pc.asm.replace{body = lid, with = thicker}
assert(#report.kept == 1 and #report.unmatched == 0)
assert(pc.doc.feature{id = j}.body == thicker, "the mate is on the new body")
local at = pc.asm.placement{body = thicker}.translation
assert(math.abs(at[3] - 5) < 1e-4, "on the base's top")
for _, b in ipairs(pc.doc.bodies()) do
  if b.id == lid then assert(not b.visible, "the old lid is hidden") end
end
```
