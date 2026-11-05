# Mesh

The Mesh bench edits a scan's mesh as a history of steps, the same way
Design builds a solid. Each step is a feature under the scan in the
tree. You can edit, delete or reorder it, and undo it like any other
edit. The history starts from the mesh as it was imported, which the
document keeps beside the edited one, so the original is never lost:
select a step in the tree and the scan shows the mesh up to there.
Inspection measures the mesh at the end of the history, so its fits,
maps and sections follow your edits.

Select a scan, then pick a step on the Mesh toolbar. Its task edits the
step live: **OK** accepts, **Cancel** restores the opening values (or
removes a newly created step). Double-click a step to edit it again.
The task lists the active history with each built step's triangle count
and shows the current step's measurements.


A mesh body's **Shading** (its Appearance task, or `pc.doc.set_body{body
= …, shading = "sharp"}`) is how its triangles are lit: *as meshed*
(welded where normals agree within 30°, as it was read), *sharp* (every
triangle flat, so a scan's noise and a tessellation's facets show), or
*smooth* across triangles turning less than a crease angle (`crease`,
30°). It changes only the look: what is measured, fitted and exported is
the mesh as it is. A colour map draws on the mesh as it is too.

## Steps

**Remove islands** takes away the small loose pieces a scanner leaves
round a part: specks, a corner of the turntable. A piece goes when its
bounding box's diagonal is shorter than **Smaller than** (a hundredth of
the scan's to start with).

**Delete marked** takes away the marked part of the scan, leaving the
hole open. Mark what to remove first; a face goes when all three of its
corners are marked.

**Flip normals** turns the marked faces over, or every face when nothing
is marked: for a scan that came in inside out.

**Cut by plane** cuts the scan square to its Z axis through its middle
and keeps what is below. With **Close** on (the default), every loop of
the cut is capped flat, holes and nested walls included. Gaps in the
cut's rim up to **Bridge gaps up to** are closed straight, and wider
gaps are left open and counted. **Offset** moves the plane along its
normal.

**Fill holes** closes rims whose longest span is no greater than **Holes
up to** (one twentieth of the mesh's bounding-box diagonal). Choose all
holes within that size or pick a point near one rim. **Curved** carries
the neighbouring surface across the hole; **Flat** makes a tight patch.
The surrounding scan vertices do not move. A picked rim larger than the
size limit is refused rather than replaced by a different hole.

**Smooth** removes scan noise from the whole mesh or a marking. **Bumps
up to** is the neighbourhood radius: initially diagonal / 125, but at
least 2.5 mean edge lengths. **Move at most** limits every point's total
movement (diagonal / 1000 initially). **Keep sharp edges** starts on at
30°. A marked region blends to zero at its boundary. The task reports
maximum and RMS movement in millimetres.

**Offset** moves the whole mesh or marked faces by a normal **Distance**,
initially 0.1 mm; negative values shrink it. This is not a scale: corners
move far enough for each incident face to offset by that distance. A
marked patch joins the rest of the mesh with a wall.

**Reduce** simplifies the mesh with a **Tolerance** (diagonal / 4000
rounded to a 1–2–5 step) and **Triangle limit** (50,000 initially). It
verifies distances in both directions and reports the share within
tolerance, the 99.9% distance, the furthest point and its distance, and
RMS. The triangle limit takes priority: the task explicitly reports
when reaching it exceeds tolerance, or when a safe reduction cannot
reach the limit.

**Boolean with solids** subtracts, adds or intersects a bounded solid:
a solid body, placed where it sits against the scan, or an Inspection
element selected before the tool (a fitted cylinder, cone or sphere, over
the extent it is drawn with). The step keeps the operand as it stood when
the step was made, meshed to **Operand tolerance** (0.01 mm). The scan
must be closed: an open one is refused with "fill holes first", and so
is an operand that does not meet it. The task gives the result's volume.

**Shell / hollow** hollows a closed part: its outer surface stays and an
inner wall **Thickness** inside it (diagonal / 125 to start with) bounds
a sealed cavity. An open sheet is thickened instead, **Inward**,
**Outward** or **Both** ways, walls closing its rims. The task gives the
result's volume.

Vertex markings follow the start's vertices through smoothing,
offsetting and reduction. A vertex Reduce collapsed stands for every
start vertex merged into it, and is marked when all of them are. Added
vertices have no start identity and are never marked.

Booleans and shells make a new mesh: a marking of the start mesh does
not reach past them either.

A step that cannot run (a delete with nothing in its region, a filter
that would remove every piece) shows its error on its row. The scan keeps
the mesh from before it.

## Deviation of the edit

**Deviation of the edit** colours the scan by how far its edits moved its
surface from the mesh as imported, signed. The scale runs to the 99th
percentile, and the legend gives the largest and the RMS movement. Surface an edit added (a cap,
a bridge) is grey, and the legend gives its share. A cut moves nothing and adds its cap. Use it to see
that an edit touched only what it meant to.
`pc.mesh.deviation{scan = s}` answers the same numbers.

## Trim to scan

A repair piece is modelled oversized, reaching into the break of the
scanned part, and **Trim to scan** (`pc.mesh.trim_to_scan`) cuts it to
fit. The break is the scan's surface lying deeper than **depth** (0.3 mm)
inside the piece; it is extended past the piece, thickened by the glue
**gap** (0.1 mm) to each side and cut away, keeping the side away from
the scan (or toward it, `side = "toward"`). The answer gives the piece's
volume, the area where it mates with the break, and the gap's median and
90 % range. A piece whose shape came from a file takes the result in
place; a piece built from its features is hidden and a new body takes
the result beside it.

## Preview and export

`pc.mesh.preview_cut` builds the cut **Cut by plane** would make on the
mesh as it stands, recording nothing, and answers its triangle count and
the loops it capped, bridged and left open. `pc.mesh.export{scan = s,
path = "part.stl"}` writes the mesh at the history's tip as STL, 3MF, PLY
or OBJ (by `format`, else the path's extension), its triangles as they
are, in the scan's frame.

## From a script

```lua
pc.mesh.islands{scan = s, smaller_than = 2}
pc.mesh.cut{scan = s, plane = "xy", at = 20, keep = "below"}
pc.mesh.delete{scan = s, ball = {centre = {10, 0, 7.5}, radius = 3}}
pc.mesh.flip{scan = s}
pc.mesh.fill{scan = s, holes_up_to = 5, fairing = "flat"}
local smoothed = pc.mesh.smooth{scan = s, move_at_most = 0.02}
pc.mesh.offset{scan = s, distance = 0.1}
local reduced = pc.mesh.reduce{scan = s, tolerance = 0.05, triangle_limit = 50000}
pc.mesh.boolean{scan = s, operand = pin, operation = "subtract"}
pc.mesh.thicken{scan = s, thickness = 1.5}
pc.doc.rebuild()
print(pc.mesh.result{feature = smoothed.feature}.report.Smooth.rms)
print(pc.mesh.result{feature = reduced.feature}.report.Reduce.p999)
print(pc.mesh.inspect{scan = s}.volume)
```

`pc.mesh.inspect` counts the triangles, open edges and pieces of the
mesh the scan draws now, with its volume and bounds. A closed mesh has
no open edges, and its volume is positive when its faces point out.
`delete`, `flip`, `smooth` and `offset` take a `box = {min = {…}, max = {…}}` or a
`ball = {centre = {…}, radius = r}` in the scan's own frame; without
either they use the scan's marking (or the whole mesh if empty, except
Delete). `mesh.fill` takes an optional `near = {x, y, z}` in the same
frame. `mesh.result{feature = step.feature}` reads a built step's
measurements; changed or unbuilt steps refuse stale results.
`mesh.set` edits an existing step's serialized `op` or cut `offset`.
