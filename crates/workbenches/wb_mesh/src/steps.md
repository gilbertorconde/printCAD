# Mesh step commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

In an example, a line `{{scan}}` stands for the setup the code hands this
file under that name.

## mesh.smooth, mesh.offset

Without a box or a ball, uses the marking, or the whole mesh if nothing is
marked.

## mesh.fill

Only holes within the size limit are filled. With near, chooses the nearest
rim before testing its size. Scan vertices never move. Rebuild before reading
mesh.result.

### Close the open top of a scan

```lua
{{scan}}

    pc.mesh.delete{scan = scan, box = {min = {-1, -1, 9.99}, max = {11, 11, 11}}}
    assert(#pc.doc.rebuild() == 0)
    assert(pc.mesh.inspect{scan = scan}.open_edges > 0)
    local step = pc.mesh.fill{scan = scan, holes_up_to = 15, fairing = "flat"}
    assert(#pc.doc.rebuild() == 0)
    assert(pc.mesh.inspect{scan = scan}.open_edges == 0)
    assert(math.abs(pc.mesh.inspect{scan = scan}.volume - 1000) < 10)
    assert(pc.mesh.result{feature = step.feature}.faces > 0)
```

## mesh.smooth

Projects points onto local quadrics in two passes. A marking blends to zero at
its edge. mesh.result reports maximum and RMS movement after rebuilding.

### Bound a smoothing step's movement

```lua
{{scan}}

    local step = pc.mesh.smooth{scan = scan, move_at_most = 0.01}
    assert(#pc.doc.rebuild() == 0)
    local r = pc.mesh.result{feature = step.feature}.report.Smooth
    assert(r.max <= 0.010001 and r.rms <= r.max)
```

## mesh.offset

Offsets faces rather than scaling the mesh. A marked patch joins its
neighbours with a wall.

### Expand a ten millimetre cube

```lua
{{scan}}

    pc.mesh.offset{scan = scan, distance = 0.1}
    assert(#pc.doc.rebuild() == 0)
    assert(math.abs(pc.mesh.inspect{scan = scan}.volume - 10.2^3) < 0.1)
```

## mesh.reduce

Verifies both ways at 99.9%. The triangle limit takes priority over tolerance;
mesh.result reports the actual within-tolerance share, p999, furthest point
and distance, RMS and whether the limit was reached. An impossible limit is
reported, not hidden.

### Read a reduction's verified accuracy

```lua
{{scan}}

    local step = pc.mesh.reduce{scan = scan, tolerance = 0.01, triangle_limit = 50000}
    assert(#pc.doc.rebuild() == 0)
    local r = pc.mesh.result{feature = step.feature}
    assert(r.faces <= 50000)
    assert(r.report.Reduce.p999 <= 0.01)
    assert(r.report.Reduce.within_share >= 0.999)
```

## mesh.remesh, mesh.subdivide

A result of more faces than face_limit is refused, the mesh before the step
kept. Markings of the start's vertices follow them through the step.

## mesh.repair_intersections

The geometry kernel cannot make this yet: the step is added and kept, its row
says why it does not build, and the mesh before it is shown until the kernel
can.

Makes a closed mesh that crosses itself whole: overlapping shells merge into
one and the surface inside them goes, as a union of the mesh with itself.
An open mesh is refused.

See also: mesh.intersections

## mesh.remesh

Evens the triangles out to edges near edge_length, the points kept on the
surface, in passes of split, collapse, flip and relax: edges settle within
about 30 % of the length, most a little under it. With keep_edges, edges
turning more than edge_angle stay as creases; open rims keep their shape.
With a box, a ball or a marking, only faces with every corner in it change.
mesh.result reports Remesh {max, rms}, how far the new surface lies from the
old.

### Remesh a cube to one millimetre edges

```lua
{{scan}}

    local step = pc.mesh.remesh{scan = scan, edge_length = 1}
    assert(#pc.doc.rebuild() == 0)
    local r = pc.mesh.result{feature = step.feature}
    assert(r.faces > 1000)
    assert(r.report.Remesh.max < 1e-3)
    assert(math.abs(pc.mesh.inspect{scan = scan}.volume - 1000) < 0.01)
```

## mesh.subdivide

Splits every triangle into four, levels times, so the count grows fourfold
each level. Without smooth the surface stays exactly as it is; with smooth
the points move toward a smooth surface each level comes nearer, open rims
and, with keep_edges, edges turning more than edge_angle kept sharp.
mesh.result reports Subdivide {moved}, the farthest an existing point moved.

### Round a cube

```lua
{{scan}}

    local step = pc.mesh.subdivide{scan = scan, levels = 3, smooth = true, keep_edges = false}
    assert(#pc.doc.rebuild() == 0)
    local r = pc.mesh.result{feature = step.feature}
    assert(r.faces == 12 * 64)
    assert(r.report.Subdivide.moved > 1)
    assert(pc.mesh.inspect{scan = scan}.volume < 1000)
```

## mesh.result

Rebuild first. Smooth reports {max, rms}; Reduce reports {within_share, p999,
furthest, max, rms, target_met}; Remesh {max, rms}; Subdivide {moved}. Distances are mm, furthest is in the body's
frame. A changed or failed history has no result for an unbuilt step.

### Read a built step

```lua
{{scan}}

    local step = pc.mesh.smooth{scan = scan, move_at_most = 0.01}
    assert(#pc.doc.rebuild() == 0)
    local result = pc.mesh.result{feature = step.feature}
    assert(result.faces > 0 and result.report.Smooth.max <= 0.010001)
```

## mesh.set

Edits the existing step; it does not create a step or open a task. Its region
uses start-mesh vertex numbers.

### Change an offset in place

```lua
{{scan}}

    local step = pc.mesh.offset{scan = scan, distance = 0.1}
    pc.mesh.set{feature = step.feature, op = {Offset = {region = "All", distance = 0.2}}}
    assert(#pc.doc.rebuild() == 0)
    assert(math.abs(pc.mesh.inspect{scan = scan}.volume - 10.4^3) < 0.1)
```
