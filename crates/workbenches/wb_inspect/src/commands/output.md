# Inspection output commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## inspect.compensation

Each scan point's distance to the model along its normal is explained as each
axis stretching about the model's middle and a leftover shift, by least
squares; points past three times the fit's RMS (support remains, defects) are
left out. `stretch` is each axis's stretch less one in percent (-0.4 a print
0.4 % small); `compensation` the factors that scale the model so the next
print comes out as drawn.

The model is read as it stood on the bed, +Z up. Scan points on the walls of
the model's round holes and bosses add each kind's radius error beyond the
stretch: `holes` and `bosses` are {radius_error, offset, points}, nil with too
few points, `offset` the radius change to draw them with (a hole printing 0.1
mm small in radius reads offset 0.1). The side walls of the first 0.6 mm are
left out of the fit; `flare` is how far they stand out past it (elephant's
foot). `warp` reads the down-facing points within 3 mm of the bottom:
{flatness, lift, at}, the bottom's peak-to-peak and how high its highest part
stands over its middle height, and where.

See also: `inspect.best_fit`

## inspect.export_step

One part holding no solid, its construction geometry named as the tree names
it: fitted and constructed planes as bounded planes, cylinders, cones and tori
as their axes and rims, spheres and points as points, circles, lines, every
section's chains and shadow outlines as curves, where the scan stands.

## inspect.export_iges

Writes the same geometry as inspect.export_step, at the scan's placement, with
the IGES construction flag (06) and no solid. `items` counts the items
submitted to the writer; `warnings` lists any geometry left out or names
carried only in part. Names containing non-ASCII characters, commas or
semicolons keep only a short label.

See also: `inspect.export_step`

### A section saved as IGES with its naming warning

```lua
local drum = pc.doc.new_body{}
pc.design.primitive{body = drum, variant = "cylinder",
  kind = {Cylinder = {radius = 5, height = 10, angle_deg = 360}}}
assert(#pc.doc.rebuild() == 0)
local path = os.tmpname() .. ".stl"
pc.file.export{path = path, bodies = {drum}}
pc.file.import{path = path, scan = true}
pc.doc.rebuild()
os.remove(path)
local scan
for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
local cut = pc.inspect.section{scan = scan}
pc.doc.rename{id = cut.section, name = "Section, centre"}
local out = os.tmpname() .. ".igs"
local saved = pc.inspect.export_iges{path = out, scan = scan}
os.remove(out)
assert(saved.path == out and saved.items == 1)
assert(#saved.warnings == 1)
assert(saved.warnings[1]:find("Section, centre", 1, true))
```
