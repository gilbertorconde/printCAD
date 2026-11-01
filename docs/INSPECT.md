# Inspection

The Inspection bench measures a scan: it fits ideal shapes to the parts
of its surface you mark, keeps each as an **element** under the scan in
the tree, and measures distances, angles and diameters between them
against a nominal and a tolerance. A scan is a mesh body marked as one (see
[Editing workflow](EDITING.md), where Mark surface… is described too).

## Fitting an element

1. Mark the stretch of the scan the shape lies on with **Mark surface…**
   in the scan's menu, or `pc.scan.mark{…}` from a script.
2. Switch to the Inspection bench and pick the shape from its toolbar:
   Plane, Sphere, Cylinder, Cone, Torus, Circle or Line. The tools stay
   off while nothing is marked.

The fit is orthogonal least squares over the marked points. Points
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

## From a script

```lua
pc.scan.mark{body = scan, box = {min = {-20, -20, 3}, max = {20, 20, 27}}}
local bore = pc.inspect.fit{kind = "cylinder"}
local size = pc.inspect.dimension{kind = "diameter", elements = {bore.element},
  nominal = 20, plus = 0.05, minus = 0.05}
print(size.value, size.pass)
```

`pc.inspect.fit{kind = …, cutoff = …}` fits the scan marked in the tab;
`cutoff` changes the three standard deviations, and `0` keeps every
point. `pc.inspect.dimension{kind = "distance", elements = {a, b},
nominal = 30, plus = 0.1, minus = 0.1}` adds a dimension.
`pc.inspect.report{scan = …}` lists every element and dimension with its
numbers, and how many dimensions passed and failed. The [scripting reference](SCRIPTING.md) has the full answers.
