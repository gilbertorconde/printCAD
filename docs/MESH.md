# Mesh

The Mesh bench edits a scan's mesh as a history of steps, the same way
Design builds a solid. Each step is a feature under the scan in the
tree. You can edit, delete or reorder it, and undo it like any other
edit. The history starts from the mesh as it was imported, which the
document keeps beside the edited one, so the original is never lost:
select a step in the tree and the scan shows the mesh up to there.
Inspection measures the mesh at the end of the history, so its fits,
maps and sections follow your edits.

Select a scan, then pick a step on the Mesh toolbar.

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

A step that cannot run (a delete with nothing in its region, a filter
that would remove every piece) shows its error on its row. The scan keeps
the mesh from before it.

## From a script

```lua
pc.mesh.islands{scan = s, smaller_than = 2}
pc.mesh.cut{scan = s, plane = "xy", at = 20, keep = "below"}
pc.mesh.delete{scan = s, ball = {centre = {10, 0, 7.5}, radius = 3}}
pc.mesh.flip{scan = s}
print(pc.mesh.inspect{scan = s}.volume)
```

`pc.mesh.inspect` counts the triangles, open edges and pieces of the
mesh the scan draws now, with its volume and bounds. A closed mesh has
no open edges, and its volume is positive when its faces point out.
`delete` and `flip` take a `box = {min = {…}, max = {…}}` or a
`ball = {centre = {…}, radius = r}` in the scan's own frame; without
either they use the scan's marking.
