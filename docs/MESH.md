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

Vertex markings follow surviving source vertices through smoothing and
offsetting. Added vertices have no start identity. The kernel cannot
carry a start marking through reduction: marked steps after Reduce are
refused. Use the whole mesh or a script's box or ball region there.

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
