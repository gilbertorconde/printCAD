# Modelling from a scan

A part that exists, scanned, drawn again as a parametric body: the scan
stays in the document as the reference, and every bench shows how the
model lies against it as the model grows. Bring the scan in with File ›
Import (tick *Scan*, or `pc.file.import{path = …, scan = true}`); align
it to the axes first if it came in skewed ([Inspection](INSPECT.md#aligning-a-scan)).

## Sketching on the scan

The sketcher cuts every visible scan by the sketch's plane and draws the
slice as a dashed guide while the sketch is open; **Fit the scan's
section** turns it into lines and arcs to constrain, and the slice shows
where the sketch strays from it. See [Sketcher](SKETCHER.md#sketching-on-a-scan).

## Deviation from scan

**Deviation from scan** (Design toolbar) paints every visible scan by
its distance to the body being modelled: the body whose feature's task
is open, else the selected body. It follows each rebuild, so a pad's
length or a sketch's dimension can be dragged until the scan turns green.

- Scan within ±0.1 mm of the body's surface ramps from blue (just
  inside) through green to red (just outside).
- Scan deeper inside the body draws violet: material the part does not
  have, a hole or pocket still to cut.
- Scan farther outside stays plain: the part's other faces, or material
  still to add.

The legend gives the share of the scan on the surface, the share inside
the body and the RMS distance of the points on it. From a script,
`pc.design.scan_deviation{body = …, scan = …, range = 0.1}` answers the
same shares: `within`, `inside`, `outside`, `rms`.

## Fitted elements as bodies

A face of the scan fitted in Inspection (a plane, cylinder, cone, sphere
or torus) becomes a body to model on: select the element and click
**Element as body** (Inspect menu). The new body holds one Design
Primitive at the fitted size: a cylinder or cone over the length its
points cover, a plane as a 2 mm slab under the patch its points cover,
a sphere or torus whole. It is an ordinary primitive from then on, its
sizes in the property panel to round off and its features to add.

From a script, `pc.inspect.element_solid{element = …}` answers the
`kind` and `placement` that `pc.design.primitive` takes:

```lua
local s = pc.inspect.element_solid{element = fit.element}
local body = pc.doc.new_body{name = s.name}
pc.design.primitive{body = body, kind = s.kind, placement = s.placement}
```

## Reading holes and fillets off the scan

**Read round off scan** (Design toolbar) arms; each click on a round hole
or boss of a scan then logs its diameter, how far it runs, its fit's σ,
and the standard size it is nearest: for a hole the tap drill or
clearance hole of a thread standard (an Ø6.6 hole reads as an M6's
normal clearance), for a boss or pin a thread's major diameter. Escape
or the button again stops it. `pc.design.read_hole{scan = …, at = {x,
y, z}}` answers the same, with the three nearest sizes and the value
the diameter was likely drawn at (`design`), to hand to
`pc.design.hole` or a sketch circle.

A round that turns less than 300° about its axis is a fillet: the click
logs its radius, the radius it was likely drawn at (to the half
millimetre), how far round it turns and whether it is an inside or an
outside round, and the command answers them as `fillet = {radius,
design, concave}`, ready for `pc.design.fillet`.

For a finished part's full deviation map, tolerances and a report, use
Inspection's [comparison](INSPECT.md#comparing-with-a-reference).
