# Sketcher file commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## sketch.import_dxf

Left out, `scale` follows the drawing's own unit: a drawing in inches comes in
at 25.4 mm a unit. A drawing that names no unit comes in at 1 mm a unit.

It is placed as `sketch.new` places a sketch: without `body` it goes in the
selected body, else a new one. It is named after the file unless `name` says
otherwise.

A file that cannot be read or parsed is refused, and so is a drawing with no
curves.

See also: `sketch.new`, `sketch.repair`

## sketch.export_dxf

At true scale, R2000, the unit declared. Lines, circles and arcs are written
as themselves, other curves as polylines within 0.01 mm, on layer ELEMENTS,
construction and projected guides dashed on CONSTRUCTION. The pictures' edges,
thinned to 0.01 mm, go dashed on PICTURE_EDGES; text blocks as TEXT on NOTES,
with a traceability line under the drawing: the scale, each picture's mm per
pixel and where it came from, and the sketch's frame. Points are left out.

See also: `sketch.export_csv`, `sketch.import_dxf`

### A circle in inches, read back

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.circle{sketch = s, x = 0, y = 0, radius = 25.4}
local path = os.tmpname()
local out = pc.sketch.export_dxf{sketch = s, path = path, units = "inch"}
assert(out.curves == 1 and out.notes == 1, out.notes)
local file = io.open(path, "r")
local text = file:read("a")
file:close()
os.remove(path)
assert(text:find("$INSUNITS\n70\n1\n", 1, true), "declared in inches")
assert(text:find("1:1 in inch", 1, true), "the traceability note")
```

## sketch.export_svg

At true scale, one unit a millimetre, every curve a polyline within 0.01 mm of
it; construction and projected guides dashed. Points are left out; the
traceability line is written under the drawing.

## sketch.export_csv

A `#` traceability line (scale, its source, frame), a header, then a row per
element (kind, id, drawn or construction, its points, radius and length) and
per dimension (kind, id, name, driving or reference, value and unit). Lengths
in mm, angles in degrees.

See also: `sketch.export_dxf`

### A line's length

```lua
local s = pc.sketch.new{plane = "XY"}
pc.sketch.line{sketch = s, x1 = 0, y1 = 0, x2 = 30, y2 = 40}
local out = pc.sketch.export_csv{sketch = s}
assert(out.csv:find(",50,,mm", 1, true), out.csv)
assert(out.csv:sub(1, 2) == "# ", "the traceability line first")
```
