# Sketcher text commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## sketch.text

A font installed on the machine, named by its family, or a font file named by
its path, is kept in the document the first time it is used, so the text is
made again from it on any machine; a font neither carried nor kept is refused
with why.

A closed curve runs clockwise from its top, so text above it stands outside
and reads across the top; `reversed` puts it inside, reading across the
bottom. The point is held on the curve: dragging it carries the text along.

A frame is four construction lines kept square to the sketch's axes; dragging
or dimensioning them sizes the room the text has.

`{formula}` in the text shows what the formula comes to, and follows it: `REV
{Part.rev}` reads the variable, `{Width * 2:1}` writes one decimal. A length
shows in millimetres, an angle in degrees; `{{` and `}}` are braces. Each
formula is kept as the sketch's under `<block>/value/<n>`.

`at` is where the baseline starts: the letters stand on it, capitals reaching
about 0.7 of `size`, the em.

The letters are closed outlines that pad as they read; dragging the point
returned moves the whole text. They are made from the block, never stored as
curves of their own, so the text stays editable with `sketch.text_edit`;
`sketch.explode_text` turns it into plain curves.

A `font` that is not one of the three bundled names is a file's path when it
looks like one (a path separator, or a font file's extension) or names a file
there, else an installed family, its letters' case aside; the family a font
gives is what the block records. Text with nothing to draw is refused.

See also: `sketch.text_edit`

### Letters padded from the baseline

```lua
local s = pc.sketch.new{plane = "XY"}
local t = pc.sketch.text{sketch = s, text = "PC", at = {0, 0}, size = 10}
assert(t.text and t.point)
local pad = pc.design.pad{sketch = s, length = 1}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs(m.min[2]) < 0.2, "the baseline is at y = 0")
assert(m.max[2] > 6 and m.max[2] < 10, "capitals stand under the 10 mm em")
```

### A revision label that follows a variable

```lua
pc.var.new{name = "Part"}
pc.var.set{set = "Part", name = "rev", formula = "3"}
local s = pc.sketch.new{plane = "XY"}
pc.sketch.text{sketch = s, text = "REV {Part.rev}", at = {0, 0}}
local pad = pc.design.pad{sketch = s, length = 1}
assert(#pc.doc.rebuild() == 0)
local body = pc.doc.feature{id = pad}.body
local before = pc.doc.measure{body = body}
pc.var.set{set = "Part", name = "rev", formula = "12"}
assert(#pc.doc.rebuild() == 0)
local after = pc.doc.measure{body = body}
assert(after.max[1] > before.max[1] + 3, "a digit longer")
```

### A label centred in a frame

```lua
local s = pc.sketch.new{plane = "XY"}
local t = pc.sketch.text{sketch = s, text = "OK", frame = {{0, 0}, {40, 20}},
  align = "middle", valign = "middle"}
assert(#t.corners == 4)
local pad = pc.design.pad{sketch = s, length = 1}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(math.abs((m.min[1] + m.max[1]) / 2 - 20) < 0.5, "centred across")
assert(math.abs((m.min[2] + m.max[2]) / 2 - 10) < 0.5, "centred up and down")
```

### Letters round a circle

```lua
local s = pc.sketch.new{plane = "XY"}
-- A guide circle: the letters are the profile.
local ring = pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 20}
pc.sketch.construction{sketch = s, items = {ring}}
pc.sketch.text{sketch = s, text = "AROUND", path = {ring}, at = {0, 30},
  align = "middle", size = 5}
local pad = pc.design.pad{sketch = s, length = 1}
assert(#pc.doc.rebuild() == 0)
local m = pc.doc.measure{body = pc.doc.feature{id = pad}.body}
assert(m.min[2] > 15, "over the circle's top")
assert(math.abs(m.min[1] + m.max[1]) < 1, "centred on the point")
```

## sketch.text_edit

`block` is either the `text` or the `point` that `sketch.text` returned.

Only what is given changes, and the outlines are made again on the same point;
a `size` of 0 or less is refused.

A block's size, spacing and angle are numbers formulas can set:
`pc.doc.parameters` of the sketch lists them under the keys `<block>/size`,
`<block>/spacing` and `<block>/angle`, and the letters stand at what they come
to. The string is not, since a formula makes a number.

See also: `sketch.text`, `doc.set_formula`

### Text whose height a variable sets

```lua
local s = pc.sketch.new{plane = "XY"}
local t = pc.sketch.text{sketch = s, text = "I", at = {0, 0}, size = 10}
pc.var.new{name = "Label"}
pc.var.set{set = "Label", name = "height", formula = "10 mm"}
pc.doc.set_formula{id = s, parameter = t.text .. "/size", formula = "Label.height"}
local pad = pc.design.pad{sketch = s, length = 1}
assert(#pc.doc.rebuild() == 0)
local body = pc.doc.feature{id = pad}.body
local small = pc.doc.measure{body = body}
pc.var.set{set = "Label", name = "height", formula = "20 mm"}
assert(#pc.doc.rebuild() == 0)
local large = pc.doc.measure{body = body}
assert(math.abs(large.max[2] - 2 * small.max[2]) < 1e-3, "twice as tall")
```

### A letter made twice as large

```lua
local s = pc.sketch.new{plane = "XY"}
local t = pc.sketch.text{sketch = s, text = "I", at = {0, 0}, size = 10}
local pad = pc.design.pad{sketch = s, length = 1}
assert(#pc.doc.rebuild() == 0)
local body = pc.doc.feature{id = pad}.body
local small = pc.doc.measure{body = body}
pc.sketch.text_edit{sketch = s, block = t.text, size = 20}
assert(#pc.doc.rebuild() == 0)
local large = pc.doc.measure{body = body}
assert(math.abs(large.max[2] - 2 * small.max[2]) < 1e-3, "twice as tall")
assert(math.abs(large.volume - 4 * small.volume) < 1e-3, "four times the area")
```

## sketch.explode_text

The curves keep the names the letters had, so what was built from the text
keeps its faces' names; the point stays, and the text is gone for good.

See also: `sketch.text`

### A letter exploded and still padded

```lua
local s = pc.sketch.new{plane = "XY"}
local t = pc.sketch.text{sketch = s, text = "L", at = {0, 0}, size = 10}
local pad = pc.design.pad{sketch = s, length = 1}
assert(#pc.doc.rebuild() == 0)
local body = pc.doc.feature{id = pad}.body
local before = pc.doc.measure{body = body}.volume
local left = pc.sketch.explode_text{sketch = s, block = t.text}
assert(#left.curves > 0)
assert(#pc.doc.rebuild() == 0)
assert(math.abs(pc.doc.measure{body = body}.volume - before) < 1e-6)
```
