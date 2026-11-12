# Inspection comparison commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## inspect.hot_spots

Vertices beyond the tolerance are joined with neighbours on the same side
across the mesh's edges. `side` is 1 outside the reference and -1 inside;
`kind` is `feature` for a patch no more than 40 times as wide as it is deep,
else `texture`. Positions are in the scan's frame.

## inspect.volume_errors

The scan is read as a solid on a grid of cubic cells and set against the
reference. `side` is `missing` where the reference has material the scan lacks
and `excess` where the scan has more; `form` is `layer` for a face standing
off, `lump` for a boss or a pocket. `volume` is in mm³, `depth` in mm, how far
apart the surfaces stand across the region. Holes in the scan read as the
reference; a region on one, or shallower than two cells, is `unsure`, and one
shallower than a cell is not reported. Positions are in the reference's frame.
The comparison keeps the cell, and the largest regions are labelled on the
part while the Inspection bench is active.

See also: `inspect.hot_spots`

### A print two millimetres short of its part reads a missing layer

```lua
local part = pc.doc.new_body{}
pc.design.primitive{body = part, kind = {Box = {length = 20, width = 20, height = 12}}}
local short = pc.doc.new_body{}
pc.design.primitive{body = short, kind = {Box = {length = 20, width = 20, height = 10}}}
assert(#pc.doc.rebuild() == 0)
local path = os.tmpname() .. ".stl"
pc.file.export{path = path, bodies = {short}}
pc.doc.set_visible{id = short, visible = false}
pc.file.import{path = path, scan = true}
pc.doc.rebuild()
os.remove(path)
local scan
for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
local c = pc.inspect.compare{scan = scan, reference = part}
local v = pc.inspect.volume_errors{comparison = c.comparison, cell = 0.5}
local r = v.regions[1]
assert(r.side == "missing" and r.form == "layer", r.side)
assert(math.abs(r.volume - 800) < 80 and math.abs(r.depth - 2) < 0.3, r.volume)
```

## inspect.map_export

Points are in the scan's own frame; an unmeasured point's distance is NaN,
blank in CSV.
