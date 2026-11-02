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

## Tolerances

**Tolerance** keeps a form or orientation tolerance with a limit; its
label on the part reads the zone it measured against the limit, PASS in
green or FAIL in red, and the property panel's **Limit** sets it.

- Select one element for its **form**: a plane's flatness, a line's
  straightness, a circle's circularity, a cylinder's cylindricity, a
  sphere's sphericity, each the band its points fill about the fit.
- Select an element and then its **datum** for an orientation
  tolerance, which the tool chooses from how they stand: **parallelism**
  and **perpendicularity** of planes and axes, **angularity** of planes
  at the basic angle they make (rounded to a degree), **coaxiality** of
  two axes, **concentricity** of a centre to an axis.
- A plane is read at its measured points, so its orientation includes
  its flatness; a cylinder, cone or line at its axis over its fitted
  length; a sphere or circle at its centre. A constructed plane is read
  at its drawn corners, and says so.
- Zones are taken about the least-squares fit, which can read a little
  wider than the narrowest zone that holds the points.

`pc.inspect.tolerance{characteristic = "parallelism", element = top,
datum = floor, limit = 0.05}` makes one from a script, and
`pc.inspect.report{}` counts dimensions and tolerances that passed and
failed together.

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

## Readings and pins

While a map shows on a scan, the cursor over it reads the map's value
there. **Pin reading** arms a click that keeps a reading on the part: a
**Pin** row under its map, its label the map's value at that point, read
again whenever the map changes (Escape or the button puts the tool
away). `pc.inspect.pin{scan = …, at = {x, y, z}}` pins one from a
script.

## Maps against one element

**Map against element** (one fitted element selected) paints the scan
by its distance from that element's ideal shape: a plane, cylinder, cone
or sphere. It needs no reference body and no alignment, and answers
"is this face flat, is this bore round?". Only scan points within the
element's drawn extent and turned within 60° of its surface are
measured; positive is the side the material faces out to, as the scan's
own normals say (a boss and a bore read the right way round), and
`flip = true` turns it over. `pc.inspect.element_map{element = …,
tolerance = 0.05}` makes one from a script.

## Aligning a scan

A scan comes in wherever the scanner put it. Alignment moves the scan
(its placement, never the reference); everything fitted or measured on
it comes along, since it is all kept in the scan's own frame. Each
alignment adds an **Alignment** row under the scan saying how it was
made, and is one undoable step.

- **Best fit to reference** moves the scan onto the selected reference
  body: point to plane, pairs past three times the median distance left
  out, starting from where the scan is and from every way of matching
  its principal axes with the reference's, so it finds the part from
  any pose. The row gives the RMS distance of the points kept; when a
  shape cannot pin the scan down (a plain cylinder turning about its
  axis, a flat plate sliding along itself) it says so.
- **Reset alignment** puts the scan back in the scanner's frame.
- `pc.inspect.move{scan = …, translate = {x, y, z}, rotate = {x, y, z}}`
  moves and turns it by numbers (about X, then Y, then Z, through the
  origin); the body menu's Placement task does the same by hand.

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

With a tolerance, the largest **hot spots** are labelled on the part:
patches where the scan stands beyond the tolerance, each its points
joined with their neighbours on the same side, read for its farthest
deviation and its area. `pc.inspect.hot_spots{comparison = …}` lists
them all, largest first, with centre, box, count, mean and extreme, each
called a *feature* (no more than 40 times as wide as it is deep: a boss,
a dent) or *texture* (layer lines, a rough skin).

`pc.inspect.map_export{comparison = …, path = …}` writes the distances,
one per scan point in the scan's frame: CSV, a binary PLY of the scan
with a `deviation` per vertex, or a NumPy array, by the path's extension
or `format`.

Both are measured where they sit, so align the scan with the reference
first (above). The map is measured again whenever either changes or moves, on a
thread of its own; the eye on the Compare row turns it off.

## Wall thickness

**Wall thickness** (Inspect menu, any bench) paints a body by how thick
it is at every point: a scan, or a part built in Design. It needs no
reference and no alignment. Select the body and click it, or call
`pc.inspect.walls{body = …, min_wall = 0.8}`.

- Each point is read inward along its normal to where the ray leaves
  the material; `sphere = true` reads the largest ball that fits there
  instead. A solid is read from its exact surfaces, a scan from its
  triangles. Surfaces nearly edge-on to the ray (within 60°) are passed
  over, and a wall thicker than a fifth of the body's diagonal (or
  `max`) is left unmeasured and grey.
- Thin is hot: the scale runs red at the thin end, between the 2nd and
  95th percentiles. With `min_wall` the legend gives the share thinner
  than it, which is how a print's minimum wall is checked.
- On a solid, points on an edge can read a wall of zero; the kernel's
  fix for it is on its way.

## Reports

**Save report…** writes the inspection as CSV: a row per element (its
form, σ, the points it kept, Ø min and max), constructed element,
dimension (reading, nominal, deviation, band, PASS or FAIL) and
tolerance (zone, limit, PASS or FAIL). `pc.inspect.report{}` answers the
same as a table, `format = "text"` as a page to read and `format =
"csv"` as CSV, and `path = …` writes it to a file in the format its
extension names; `scan = …` keeps to one scan.

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
`pc.inspect.report{scan = …}` lists every element, constructed element,
dimension and tolerance with its numbers, and how many passed and
failed (Reports, above). The [scripting reference](SCRIPTING.md) has the full answers.
