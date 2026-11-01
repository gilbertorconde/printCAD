# Inspection

The Inspection bench measures a scan: it fits ideal shapes to the parts
of its surface you mark and keeps each as an **element** under the scan
in the tree. A scan is a mesh body marked as one (see
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

## From a script

```lua
pc.scan.mark{body = scan, box = {min = {-20, -20, 3}, max = {20, 20, 27}}}
local bore = pc.inspect.fit{kind = "cylinder"}
print(bore.shape.Cylinder.radius * 2, bore.form)
```

`pc.inspect.fit{kind = …, cutoff = …}` fits the scan marked in the tab;
`cutoff` changes the three standard deviations, and `0` keeps every
point. `pc.inspect.report{scan = …}` lists every element with its
numbers. The [scripting reference](SCRIPTING.md) has the full answers.
