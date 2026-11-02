# Inspection

The Inspection bench measures a scan: it fits ideal shapes to the parts
of its surface you mark, keeps each as an **element** under the scan in
the tree, builds points, lines, planes and circles from them, measures
distances, angles and diameters between them against a nominal and a
tolerance, and paints the scan by how far it stands from the body it
should match. A scan is a mesh body marked as one (see
[Editing workflow](EDITING.md), where Mark surface… is described too).

## Fitting an element

The quickest way is a click. Switch to the Inspection bench, pick a
shape from its toolbar while nothing is marked, and click the scan where
that shape is. The fit starts at the
triangle under the cursor and spreads to each neighbour that lies on
the surface it has fitted so far, fitting again as it grows, so a bore
stops at the plate around it and a face stops at its edges. A Circle or
a Line is traced instead along the sharp edge nearest the click (a
hole's rim, a box's edge), as far as the edge runs on without turning
past 30° or meeting another; on a rounded edge, fit the faces on either
side and construct the line where they meet. The tool
stays pressed for the next click; Escape or the same button puts it
away. A click on a surface of another kind is refused with a note in
the log.

To fit exactly the points you choose:

1. Mark the stretch of the scan the shape lies on with **Mark surface…**
   in the scan's menu, or `pc.scan.mark{…}` from a script.
2. Pick the shape from the bench's toolbar: Plane, Sphere, Cylinder,
   Cone, Torus, Circle or Line. With something marked, the tool fits
   what is marked at once.

Either way, the fit is orthogonal least squares. Points
farther than three standard deviations from the shape are left out and
the shape fitted again, until the set it keeps stops changing, so a
stray speck or the edge of a neighbouring face does not pull it. The
element is named after its shape ("Cylinder 1") and draws as lines over
the scan where it was fitted: a plane's patch, a cylinder's or cone's
rims and four lines along it, a sphere's three rings, a circle, a
torus's middle ring, a line's stretch.

Its tree row's tip and the log give the result:

- **Size**: a diameter for spheres, cylinders and circles, the included
  angle for a cone, both diameters for a torus.
- **σ**: the RMS distance of the points used from the shape, in mm.
- **form**: the width of the band the points fill around the shape, in
  mm, the nearest-inside to the farthest-outside point.

A round element also knows the smallest and largest diameter its points
reach. Everything is measured in the scan's own frame, so moving the
scan moves its elements with it.

## Constructed elements

Elements can be made from other elements, fitted or constructed. Select
them in the tree (Ctrl for several) and pick **Construct point**,
**Construct line**, **Construct plane** or **Construct circle**; each
lights up when the selection can make it.

- **Point**: the midpoint of two centres, or where an axis crosses a
  plane.
- **Line**: through two centres, or where two planes meet.
- **Plane**: through three centres, midway between two planes, or offset
  from one plane (its **Offset** in the property panel's Parameters).
- **Circle**: where a plane cuts a sphere, or a cylinder within 5° of
  square to its axis (further over, the cut is an ellipse and is
  refused).

Wherever a centre is asked, a sphere, circle or torus gives its centre;
wherever an axis is asked, a cylinder, cone, circle or line gives its
axis. A constructed element keeps only what it is made from, so it
follows them when they are refitted, and dimensions measure it as they
measure a fitted one. Deleting an element deletes everything made or
measured from it.

## Dimensions

Select one element in the tree, or two with Ctrl, and pick **Distance**,
**Angle** or **Diameter** from the bench's toolbar (each lights up when
the selection fits). The dimension goes under its first element and
reads its elements again whenever one is refitted; deleting an element
deletes the dimensions on it.

- **Distance** is measured from the middle of the first element. Between
  two centres (spheres, circles, tori) it also gives ΔX, ΔY and ΔZ in the
  scan's frame. From a centre to an axis it is square to the axis; to a
  plane it is signed along the plane's normal. Two axes within 3° of
  parallel give their offset, otherwise where they come closest. Planes,
  or an axis and a plane, more than 3° from parallel get no distance:
  measure their angle. Between two planes it also gives the **range**
  their surfaces span once each one's form is counted, the smallest and
  largest a caliper would find.
- **Angle** between axes, or an axis and a plane, reads 0–90°; between
  two planes it is the angle between their normals, 0–180°.
- **Diameter** of a sphere, cylinder or circle, with the smallest and
  largest its points reach.

A dimension starts with its **nominal** at the round value nearest its
first reading (to the half millimetre under 10 mm, the millimetre above,
five degrees for an angle) and no tolerance. Set **Nominal**, **Plus**
and **Minus** in the property panel's Parameters (formulas work there
too); once Plus or Minus is above zero the label on the part shows the
deviation and **PASS** or **FAIL**, green or red. The labels show while
the Inspection bench is active, moved apart so none covers another.

## Comparing with a reference

**Compare with reference** measures how far every point of a scan stands
from a body it should match: a part built in Design, an imported STEP
or another mesh. Select the reference in the tree (with one scan in the
document) and click it, or call `pc.inspect.compare{scan = …, reference
= …}`. A **Compare** row goes under the scan, and while the Inspection
bench is active the scan is painted by its distance from the reference
and the reference draws see-through:

- A distance is positive where the scan stands outside the reference
  and negative inside. Each scan point is measured to the nearest
  reference surface turned within 90° of its own, so a thin wall's two
  sides are not mixed up.
- A solid reference is measured as its exact surfaces meshed far finer
  than anything measured (a hundredth of a millimetre on a 100 mm part);
  a mesh reference as its triangles.
- The colour scale runs from −range (blue) through zero (green) to
  +range (red), past either end in darker caps, and grey where a point
  was not measured. The range starts at the 95th percentile of the
  distances rounded up to 1, 2 or 5, and never wider than 1 mm.
- The legend at the bottom left gives the scale, a histogram of the
  distances along it, the mean, RMS, σ, smallest and largest, and with a
  tolerance the share within it.

Both are measured where they sit, so align the scan with the reference
first. The map is measured again whenever either changes or moves, on a
thread of its own; the eye on the Compare row turns it off.

## From a script

```lua
pc.scan.mark{body = scan, box = {min = {-20, -20, 3}, max = {20, 20, 27}}}
local bore = pc.inspect.fit{kind = "cylinder"}
local size = pc.inspect.dimension{kind = "diameter", elements = {bore.element},
  nominal = 20, plus = 0.05, minus = 0.05}
print(size.value, size.pass)
```

`pc.inspect.grow{kind = …, scan = …, at = {x, y, z}, peel = …}` fits
where a point lands, as a click does (`peel` takes a face's rounds off
its rim); `pc.inspect.fit{kind = …, cutoff = …}` fits the scan marked in
the tab;
`cutoff` changes the three standard deviations, and `0` keeps every
point. `pc.inspect.construct{made = "plane", from = {a, b}}` makes a constructed
element. `pc.inspect.dimension{kind = "distance", elements = {a, b},
nominal = 30, plus = 0.1, minus = 0.1}` adds a dimension.
`pc.inspect.report{scan = …}` lists every element and dimension with its
numbers, every constructed element with its shape, and how many
dimensions passed and failed. The [scripting reference](SCRIPTING.md) has the full answers.
