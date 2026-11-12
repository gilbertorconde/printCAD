# Assembly parts list commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## asm.parts

Bodies count as one part when they share one shape: linked copies from
`asm.copy` do; bodies modelled apart do not, however alike.

`size` is the part's bounding box, mm.

`threads` are one piece's cosmetic threads (a hole's that shows its thread
rather than cutting it), counted by designation such as M6.

`volume` and `mass` are of one piece: the volume the kernel measures of its
solid (a mesh body's from its triangles), the mass at its body's material's
density, else at the list's printing material (`asm.print_material`). `print`
is how many to print: the count set with `asm.part`, else one per body, none
of a bought part.

See also: `asm.part`, `asm.parts_table`, `asm.copy`

### A plate and its copies counted as one part

```lua
local function box(x)
  local s = pc.sketch.new{plane = "XY"}
  pc.sketch.rect{sketch = s, x = x, y = 0, width = 10, height = 10}
  return pc.doc.feature{id = pc.design.pad{sketch = s, length = 2}}.body
end
local plate = box(0)
local other = box(20)
assert(#pc.doc.rebuild() == 0)
pc.asm.copy{body = plate, count = 2}
local parts = pc.asm.parts{}
assert(#parts == 2, "alike bodies modelled apart are two parts")
local counts = {}
for _, p in ipairs(parts) do counts[p.quantity] = p end
assert(counts[3] and counts[3].size[3] == 2,
  "the plate and its two copies: one part, three of it")
```

## asm.part

What is set is kept for the whole part, every body of its shape, whichever
body is named.

A `number` of 0 or less takes the number away. A value that is not text is
kept as text: 3 becomes "3".

See also: `asm.parts`, `asm.parts_table`

### A bought screw numbered on one of its copies

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
local screw = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
assert(#pc.doc.rebuild() == 0)
local copies = pc.asm.copy{body = screw, count = 3}
pc.asm.part{body = copies[2], number = 4, bought = true,
  values = {Supplier = "ACME"}}
local p = pc.asm.parts{}[1]
assert(p.quantity == 4 and p.number == 4 and p.bought,
  "set on one, kept for the part")
assert(p.values.Supplier == "ACME")
```

## asm.print_material

It is kept with the document. A body with a material of its own
(`doc.set_body`) is weighed at that material's density instead.

A name the list does not offer needs its density.

See also: `asm.parts`, `asm.part`

### A plate's filament in PETG, two to print

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.rect{sketch = s, x = 0, y = 0, width = 20, height = 10}
local plate = pc.doc.feature{id = pc.design.pad{sketch = s, length = 5}}.body
assert(#pc.doc.rebuild() == 0)
local m = pc.asm.print_material{name = "petg"}
assert(m.name == "PETG" and math.abs(m.density - 1.27) < 1e-6)
pc.asm.part{body = plate, print = 2}
local p = pc.asm.parts{}[1]
assert(p.print == 2 and math.abs(p.volume - 1000) < 1e-3)
assert(math.abs(p.mass - 1.27) < 1e-4, "1 cm³ at 1.27 g/cm³")
```

## asm.parts_table

It replaces the whole list: a number, bought mark or value the table leaves
out is gone.

An empty Lua table goes as a list and `entries` refuses it: leave `entries`
out for none.

See also: `asm.part`, `asm.parts`

### The parts list written whole

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 3}
local screw = pc.doc.feature{id = pc.design.pad{sketch = s, length = 10}}.body
assert(#pc.doc.rebuild() == 0)
pc.asm.part{body = screw, number = 9, values = {Note = "old"}}
pc.asm.parts_table{table = {
  columns = {"Supplier"},
  entries = {[screw] = {number = 2, bought = true, values = {Supplier = "ACME"}}},
}}
local p = pc.asm.parts{}[1]
assert(p.number == 2 and p.bought and p.values.Supplier == "ACME")
assert(p.values.Note == nil, "what the table left out is gone")
```
