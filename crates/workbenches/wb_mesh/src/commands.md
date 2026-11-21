# Mesh clean-up commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## mesh.delete

Without a box or a ball it deletes the marked part of the scan. A face goes
when all three of its corners are in the region.

## mesh.flip

Without a region it turns the marked faces over, or every face when nothing is
marked.

## mesh.cut

The plane is xy, xz or yz, or any plane given by its `normal`. It passes
through `origin`, or else through the point nearest the mesh's middle that
lies `at` mm along the normal from the body's origin (for xy, the height z =
`at`), or through the middle itself. Above is the side the normal points to.

## mesh.split

Splits the mesh the scan draws now, its history applied, into one new mesh
body per piece: triangles joined where they share a corner, as mesh.inspect
counts pieces. The bodies are named after the scan and numbered from 1, take
its placement, look and material, and stand where it stood; the scan is
hidden. One undo takes it all back. A mesh of one piece is refused.

### Split two parts scanned together

```lua
    local s = pc.sketch.new{plane = "XY"}
    pc.sketch.rect{sketch = s, x = 0, y = 0, width = 10, height = 10}
    pc.sketch.rect{sketch = s, x = 20, y = 0, width = 5, height = 5}
    pc.design.pad{sketch = s, length = 10}
    assert(#pc.doc.rebuild() == 0)
    local path = pc.file.temp_path{name = "parts.stl"}
    pc.file.export{path = path, format = "stl"}
    pc.file.import{path = path, scan = true}
    local scan
    for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
    assert(pc.mesh.inspect{scan = scan}.pieces == 2)
    local split = pc.mesh.split{scan = scan}
    assert(#split.bodies == 2)
    local a = pc.mesh.inspect{scan = split.bodies[1]}.volume
    local b = pc.mesh.inspect{scan = split.bodies[2]}.volume
    assert(math.abs(math.min(a, b) - 250) < 1 and math.abs(math.max(a, b) - 1000) < 1)
```

## mesh.intersections

Of the mesh the scan draws now: every pair of its triangles that meet away
from the corners they share, counted as `pairs`, and the triangles in them
by number. The tool shows them on the selected scan, their corners coloured
over grey. Copies of one corner at a crease count as one corner, and
triangles without area are left out.

See also: mesh.repair_intersections

## mesh.deviation

Each point of the mesh the scan draws now is measured to its mesh as imported:
`max` and `rms` in mm, `p99` the 99th percentile, and `added` the share of
points on surface an edit added (caps, bridges: a triangle whose middle lies
off the start). The tool shows it as a map, added surface grey.

## mesh.inspect

Of the mesh the body draws now, its history applied. A closed mesh has no open
edges; its volume is positive when its faces point out.
