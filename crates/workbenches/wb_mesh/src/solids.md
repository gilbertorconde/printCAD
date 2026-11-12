# Mesh solid commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

In an example, a line `{{scan}}` stands for the setup the code hands this
file under that name.

## mesh.boolean

Takes the operand as it stands when the step is made: a body's solid where it
sits, or an Inspection element's cylinder, cone or sphere over its drawn
extent. The scan must be closed (fill holes first), and an operand it does not
meet is refused when the step builds. mesh.result reports cut_triangles and
the volume in mm³.

### Drill a scan

```lua
{{scan}}

    local pin = pc.doc.new_body{}
    pc.design.primitive{body = pin, kind = {Cylinder = {radius = 1, height = 12, angle_deg = 360}}, placement = {origin = {5, 5, -1}, x_axis = {1, 0, 0}, z_axis = {0, 0, 1}}}
    assert(#pc.doc.rebuild() == 0)
    local step = pc.mesh.boolean{scan = scan, operand = pin}
    assert(#pc.doc.rebuild() == 0)
    local volume = pc.mesh.result{feature = step.feature}.report.Boolean.volume
    assert(math.abs(volume - (1000 - math.pi * 10)) < 1)
```

## mesh.thicken

A closed mesh keeps its outer surface and gains an inner wall around a sealed
cavity. An open sheet gains a second layer joined to it by walls along its
rims; both puts half the thickness to each side. mesh.result reports whether a
cavity was made and the volume in mm³.

### Wall a cube in

```lua
{{scan}}

    local step = pc.mesh.thicken{scan = scan, thickness = 1}
    assert(#pc.doc.rebuild() == 0)
    local r = pc.mesh.result{feature = step.feature}.report.Shell
    assert(r.hollow and math.abs(r.volume - 488) < 10)
    local walled = pc.mesh.inspect{scan = scan}
    assert(walled.open_edges == 0 and math.abs(walled.volume - r.volume) < 1e-3)
```

## mesh.trim_to_scan

The break is the scan's surface lying deeper than depth inside the body; it is
extended past the body, thickened by the gap to each side and cut away,
keeping the side given. A body whose shape came from a file takes the result
in place; a body built from its features is hidden and a new body takes it.
Volume in mm³, mating area in mm², the gap's median and 90 % range in mm.
Changing the shape clears the undo history, as an import does.

### Trim a cap to a broken top

```lua
{{scan}}

    local cap = pc.doc.new_body{}
    pc.design.primitive{body = cap, kind = {Box = {length = 12, width = 12, height = 8}}, placement = {origin = {-1, -1, 8}, x_axis = {1, 0, 0}, z_axis = {0, 0, 1}}}
    assert(#pc.doc.rebuild() == 0)
    local r = pc.mesh.trim_to_scan{scan = scan, body = cap, gap = 0.1}
    assert(math.abs(r.volume - 144 * 5.9) < 0.01)
    assert(math.abs(r.mating_area - 100) < 0.01 and math.abs(r.gap_median - 0.1) < 1e-6)
```

## mesh.preview_cut

Builds the cut mesh.cut would add on the mesh as it stands, recording nothing.

### Preview a cap

```lua
{{scan}}

    local before = pc.mesh.inspect{scan = scan}.triangles
    local cut = pc.mesh.preview_cut{scan = scan, at = 5}
    assert(cut.faces > 0 and cut.report.Cut.capped_loops == 1)
    assert(cut.report.Cut.open_loops == 0)
    assert(pc.mesh.inspect{scan = scan}.triangles == before)
```

## mesh.export

Writes the mesh at its history's tip, its triangles as they are, in the body's
frame and millimetres.

### Export and read back

```lua
{{scan}}

    local path = os.tmpname() .. ".ply"
    local result = pc.mesh.export{scan = scan, path = path}
    pc.file.import{path = path, scan = true}
    os.remove(path)
    assert(result.triangles == pc.mesh.inspect{scan = scan}.triangles)
```
