# Inspection

The Inspection bench measures a scan: it fits ideal shapes to the parts
of its surface you mark, keeps each as an **element** under the scan in
the tree, builds points, lines, planes and circles from them, measures
distances, angles and diameters between them against a nominal and a
tolerance, and paints the scan by how far it stands from the body it
should match. A scan is a mesh body marked as one (see
[Editing workflow](EDITING.md), where Mark surface… is described too).

Readings show in the document's unit: dimension and tolerance labels,
pins, map legends and their scales, with four decimals in inches where
millimetres have three. What is stored, written to files and answered to
commands and agents stays in millimetres, as do the report's columns.

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

A script's `method` fits the points least squares kept another way:
`minimum_zone` to the middle of the narrowest zone holding them,
`inscribed` to the largest circle or cylinder clear of them (a hole's
mating size, what a pin through it can be) and `circumscribed` to the
smallest round them (a boss's). `pc.inspect.grow{kind = "cylinder",
scan = …, at = {…}, method = "inscribed"}`.

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
  sphere's sphericity, each the width of the narrowest zone holding its
  points (the minimum zone).
- Select an element and then its **datum** for an orientation
  tolerance, which the tool chooses from how they stand: **parallelism**
  and **perpendicularity** of planes and axes, **angularity** of planes
  at the basic angle they make (rounded to a degree), **coaxiality** of
  two axes, **concentricity** of a centre to an axis.
- A plane is read at its measured points, so its orientation includes
  its flatness; a cylinder, cone or line at its axis over its fitted
  length; a sphere or circle at its centre. A constructed plane is read
  at its drawn corners, and says so.
- Form is the minimum zone: exactly so for flatness and circularity,
  and for straightness, cylindricity and sphericity the narrowest zone
  near the least-squares one, said beside the label where it is not
  certified the narrowest. An element that keeps no zone reads its
  least-squares band, and says so. Orientation zones are
  taken about the least-squares fit.

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

- **3-2-1 datums** takes three elements selected in tree order: a primary
  plane or cylinder/cone/line axis, a secondary plane normal or axis, and
  a zero point, sphere/circle centre or plane. The primary faces −Z by
  default (the part stands on it), and the secondary turns along +X
  without disturbing the primary. Each plane zeroes its remaining
  coordinate; three perpendicular planes put their corner at the origin.
  Any coordinate left free is centred on the scan. From a script,
  `pc.inspect.align_datums{primary = a, secondary = b, zero = c}` accepts
  `primary_side` and `secondary_along` as signed axes (`"+x"`, `"-z"`,
  and so on), `zero_at = {x, y, z}`, and `centre = false` to preserve
  free translation instead. Parallel directions or elements on different
  scans are refused.
- **Auto-align** reads the scan alone: its largest flat face or planar
  open rim goes down, its long side along X, and the bounding-box centre
  on the standing face goes to the origin. Predominantly round walls
  take their common axis as Z. The alignment row says what it rests on,
  including when it falls back to principal axes. Select the scan and
  click the tool, or call `pc.inspect.auto_align{scan = …}`.
- **Use symmetry** finds a mirror plane from the scan’s principal planes,
  turns its normal to the nearest signed world axis with the smallest
  rotation, and shifts it through the origin without a tangential move.
  A mirror RMS above 0.5 mm is refused. Select the scan and click the
  tool, or call `pc.inspect.use_symmetry{scan = …}`.
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


**Local fine fit.** `pc.inspect.best_fit{scan = …, reference = …,
marked = true}` fits the scan's marked part alone: mark the face or
region the part should sit on (a datum face, a mounting pad), and the
rest of the scan follows it, free to stray where the part is worn or
bent. The alignment row says it was fitted on the marked part.

**By picked points.** `pc.inspect.align_points{scan = …, pairs = {{scan
= {x, y, z}, reference = {x, y, z}}, …}}` moves the scan so three or
more of its points land on their partners on a reference as nearly as
they can (least squares), and says how nearly (`rms`). Pick features
both have, well apart (corners, hole centres): a scan far from where it
belongs comes close enough for a Best fit to finish the job.

## Comparing with a reference

**View › Split view** shows the scans and the other bodies side by side
under one camera: the scans in the viewport on the left, where picking,
marking and every tool work, the rest in a pane on the right that
follows every orbit, pan and zoom. It is on only while the document has
a scan.

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

**Volume errors** compare the two as solids rather than surfaces:
`pc.inspect.volume_errors{comparison = …, cell = …}` reads the scan as a
solid on a grid of cubic cells (a two-hundredth of the scan's longest
side when no cell is given) and lists the regions where it and a solid
reference differ, largest first: *missing* where the reference has
material the scan lacks, *excess* where the scan has more, each a
*layer* (a face standing off) or a *lump* (a boss or a pocket), with its
volume in mm³ and its depth, how far apart the surfaces stand across it.
This finds what a surface map cannot, such as a missing boss behind a
wall. Holes in the scan read as the reference; a region on one, or
shallower than two cells, is marked unsure, and one shallower than a
cell is not reported. The comparison keeps the cell, and the largest
regions are labelled on the part while the bench is active; `off = true`
stops reading them.

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

## Fit check

**Fit check** (Inspect menu) maps how much room a part has in what it
fits into: a scanned shaft or device against the housing designed for
it, before the housing is printed. Place the part where it goes first
(an alignment, the Assembly's joints or a placement); the two are
measured as they sit. Select the scan and click it (the solid is found
when there is one), or call `pc.inspect.clearance{part = …, housing = …,
min_gap = 0.3}`.

- Each of the part's points reads its distance to the housing's
  surface: positive across a gap, negative inside the housing's
  material, where the two interfere. Points farther than `reach` (5 mm)
  stay grey.
- Interference is hot: red where the part presses in, blue where it has
  room. The legend gives the share interfering and how deep, the
  narrowest gap when nothing interferes, and with `min_gap` the share
  with less room than that.
- The map follows either body as it moves or is rebuilt, so the housing
  can be edited in Design until the fit is right.

## Overhangs

**Overhangs** (Inspect menu) paints a body by how far its surface leans
out past vertical as it stands, +Z up: a wall reads 0°, a ceiling 90°.
Surface facing up, and the face the body stands on at its lowest point,
stay grey. The legend gives the share of the surface past the printer's
limit, which needs support, and the steepest overhang. Select the body
and click it, or call `pc.inspect.overhang{body = …, limit = 50}`; the
limit is 45° when left out, and a formula in the property panel changes
it later. Turn the body (its placement) to see another way of printing
it; the map follows.

## Sections

**Section** cuts a scan by a plane and draws the cut on it, labelled
with the chains' length and, where they close, the area of material
inside them (holes taken away). Select a fitted plane to cut along it,
a cylinder, cone, line, circle or torus to cut across its axis, or the
scan itself for a cut square to Z through its middle.

- The plane is kept in the scan's own frame as it is when made:
  aligning the scan carries it, refitting the element it came from does
  not move it.
- **Offset**, **Cuts** and **Step** (the section's parameters, formulas
  allowed) move the plane along its normal and make a stack of cuts,
  each labelled with its offset: a taper, a draft or a round's run reads
  off the stack.
- A stack follows each closed loop from cut to cut and reads how it
  runs: its **draft** (the wall's angle from the normal, positive where
  it widens), its **tilt** (its centres' line from the normal), its
  circle-fitted radius at each end, and how round it is. The top cut's
  label gives them, and `pc.inspect.section{…}.tracks` lists every
  loop's.
- `through = true` with an axis makes a half-section: the plane
  contains the axis, turned `angle` degrees about it, and only the side
  it is turned to is kept, so it reads radius against height.
- Chains shorter than 1 mm are dropped as noise.
- A section is somewhere to sketch: select it (or a fitted plane) and
  use the Sketcher's Create sketch. The new sketch sits on its first
  cut, in a body of its own, with the scan's slice as its guide.

`pc.inspect.section{scan = …, plane = "xz"}` cuts along a coordinate
plane; `element = …, offset = …, count = …, step = …` stacks cuts
across an element.

## Shadow outlines

**Shadow outline** draws the outline of everything the scan covers seen
along a direction, clear of it by a play, and stands it up as a block:
the pocket a part drops into, a foam insert, a bin. Select a fitted
plane or an axis to look along it, or the scan to look straight down.

- The outline is lines, arcs and circles. Every point of it lies
  between **Play** (1 mm; 0 is exact, negative a press fit) and Play +
  2 × **Tolerance** (0.1 mm) outside the shadow, so the part fits
  whichever way the outline was fitted. Corners turning less than
  **Round below** (30°) take the largest arc that keeps to the band, and
  a hole through the part stays a hole, unless it is no wider than
  **Fill below** (0, off): holes and notches up to that across are
  filled, the outline's inner corners rounded to half of it, and the
  band is kept about the filled shadow, so the outline still clears the
  part.
- The label gives the loops, the shadow's area and the clearance the
  outline keeps, least and largest.
- The outline lives on a body of its own, whose solid is the block: the
  part's height along the direction, or `height`. File › Export writes
  it like any solid.
- It follows the scan: aligning the scan, or changing Play, Tolerance,
  Round below or Fill below, works the outline out again.

`pc.inspect.shadow{scan = …, plane = "xy", play = 0.5, height = 8}`;
`block = false` keeps the outline without a block.

## Reports

**Save report…** writes the inspection as CSV: a row per element (its
form, σ, the points it kept, Ø min and max), constructed element,
dimension (reading, nominal, deviation, band, PASS or FAIL) and
tolerance (zone, limit, PASS or FAIL). `pc.inspect.report{}` answers the
same as a table, `format = "text"` as a page to read and `format =
"csv"` as CSV, and `path = …` writes it to a file in the format its
extension names; `scan = …` keeps to one scan.


**A printable report.** `pc.doc.report_pdf{path = "report.pdf"}` writes
an A4 PDF: the tally, pictures of the scans from the isometric, top and
front views, and every element, dimension and tolerance in a table.
The maps the view shows when it is written (a comparison, wall
thickness, a fit check, an element's map) paint the scans in its
pictures, and a **Maps** section gives each one's scale and readout with
its body pictured from the same three views. The assistant's `view`
tool and `pc.doc.picture` paint them as well.

## Semantic PMI on a reference

**File › Export › STEP** (including STEP (NURBS)) writes the tolerances,
datums and dimensions of each scan compared with an exported reference
body. These are AP242 semantic annotations attached to the reference's
faces, not just drawn labels. Align the scan to the reference and create
a **Comparison** before exporting the reference. The scan need not be
visible or included in the export. `file.export` writes the same PMI.

Tolerances carry their zone limits and datum references. Distinct datum
elements receive A, B, C… in order of first use on each reference; repeated
uses keep the same letter. An export of several references adds numeric
suffixes where letters clash, keeping each tolerance on its own datum.
Circularity uses the AP242 word *roundness*; sphericity is a spherical
surface-profile tolerance. An angularity also carries its basic angle as
an angular dimension. Distance, diameter and angle dimensions carry their
nominal values and signed plus/minus allowances, with separate face groups
for the two ends of a distance or angle.

Face association uses points on the aligned scan's measured patches (or
on a constructed element's surface). Export reports an error if an
annotation cannot find a nearby reference face; it does not write an
unattached annotation. STL and 3MF carry no PMI.

## Elements as STEP or IGES

**Save elements as STEP…** writes the inspection's geometry as one part
that holds no solid, its construction geometry named as the tree names
it: fitted and constructed planes as bounded planes, cylinders, cones
and tori as their axes and rims, spheres and points as points, circles,
lines, every section's chains and every shadow outline as curves, all
where the scan stands. Another program reads them as datums to model
against. `pc.inspect.export_step{path = …, scan = …}` writes the same.

**Save elements as IGES…** writes the same elements, sections and outlines
with the IGES construction flag (06), not as solids. The dialog saves an
`.igs` file; `pc.inspect.export_iges{path = …, scan = …}` also accepts an
`.iges` path. Both commands take an optional `scan` to limit the export.
The IGES command returns `{path, items, warnings}`: `items` counts the
geometry submitted to the writer, and `warnings` reports any geometry
left out or names carried only in part. Names with non-ASCII characters,
commas or semicolons keep only a short label. The toolbar action shows
these warnings in the log.

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
