# Inspection element commands: notes and examples

Each `## id` section adds its paragraphs to that command as notes, a
`See also:` line as related commands and each `### title` with its `lua`
block as an example, which runs in the tests and must end in `assert`s. A
section naming several commands goes to each of them, before their own.
The command reference in `docs/SCRIPTING.md` is generated from them.

## inspect.compare

A distance is positive where the scan stands outside the reference, negative
inside. A solid reference is measured exactly as it is meshed for the purpose,
finely; a mesh one as its triangles. Both are measured where they sit, so
align the scan first. The map is drawn on the scan while the Inspection bench
is active.

## inspect.construct

A point is two centres' midpoint or where an axis crosses a plane; a line runs
through two centres or where two planes meet; a plane runs through three
centres, midway between two planes or offset from one; a circle is where a
plane cuts a sphere, or a cylinder within 5° of square. A sphere, circle or
torus gives its centre, a cylinder, cone, circle or line its axis. The
construction follows its sources when they are refitted, and dimensions take
it as they take a fitted element.

## inspect.fit

It fits what is marked (`pc.scan.mark`, or Mark surface… in the scan's menu),
by orthogonal least squares. `sigma` is the RMS distance of the points used
from the shape, `form` the width of the band they fill, `diameters` the
smallest and largest a round shape reaches.

Least squares picks the points used; `method` then fits those another way:
`minimum_zone` to the middle of the narrowest zone holding them (a plane,
line, circle, cylinder or sphere), `inscribed` to the largest circle or
cylinder clear of them (a hole's mating size), `circumscribed` to the smallest
round them (a boss's). `zone` is the minimum zone's width of the points used,
{width, optimal}, the form a flatness, straightness, circularity, cylindricity
or sphericity tolerance reads; `optimal` is false where the zone of a line,
cylinder or sphere is only certified locally.

## inspect.dimension

A distance is measured from the middle of the first element: between centres
(spheres, circles) with `deltas` along X, Y and Z; from a centre to an axis or
a plane; between near-parallel axes as their offset, otherwise where they come
closest; between planes or an axis and a plane only within 3° of parallel,
planes with the `range` their surfaces give. An angle between axes or an axis
and a plane reads 0–90°, between planes' normals 0–180°. `pass` is there once
`plus` or `minus` is above zero. A dimension reads its elements again whenever
they change; `doc.set_value` sets its nominal, plus and minus.

## inspect.grow

The region starts at the triangle nearest `at` and takes each neighbour whose
points lie on the fitted surface and whose normal agrees with it, fitting
again as it grows. A circle or a line is traced instead along the sharp edge
nearest `at` (a hole's rim, a box's edge) until the edge turns away or meets
another. It is refused where what is at `at` is not of the kind asked.

### A scanned bore's mating size sits inside its mean size

```lua
local drum = pc.doc.new_body{}
pc.design.primitive{body = drum, variant = "cylinder",
  kind = {Cylinder = {radius = 5, height = 10, angle_deg = 360}}}
assert(#pc.doc.rebuild() == 0)
local path = os.tmpname() .. ".stl"
pc.file.export{path = path, bodies = {drum}}
pc.doc.set_visible{id = drum, visible = false}
pc.file.import{path = path, scan = true}
pc.doc.rebuild()
os.remove(path)
local scan
for _, b in ipairs(pc.doc.bodies()) do if b.scan then scan = b.id end end
local mean = pc.inspect.grow{kind = "cylinder", scan = scan, at = {5, 0, 5}}
local inside = pc.inspect.grow{kind = "cylinder", scan = scan,
  at = {5, 0, 5}, method = "inscribed"}
assert(inside.method == "inscribed")
assert(inside.shape.Cylinder.radius <= mean.shape.Cylinder.radius + 1e-9)
assert(mean.zone.width <= mean.form + 1e-9, mean.zone.width)
```
