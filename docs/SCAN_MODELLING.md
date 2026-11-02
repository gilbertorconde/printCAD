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

## Reading threads off the scan

**Read thread off scan** (Design toolbar) arms the same way; a click on a
thread, in a hole or on a bolt, logs its pitch, hand, major and minor
diameters and the standard thread it is nearest. The thread's axis comes
from a cylinder grown over its crests and flanks, then every scan point
near it is read for the helix it follows. `clarity` says how clearly the
points follow it: near 1 on a clean scan of a thread, low where there is
none. `pc.design.read_thread{scan = …, at = {x, y, z}}` answers the same
with the three nearest standards, ready for `pc.design.hole`'s `thread`.

## Snapping faces to the scan

Draw near, then snap: select a face of the model, arm **Snap face to
scan** (Design toolbar) and click the scan where that face should lie. A
plane or cylinder, as the face is, is fitted to the scan there; a flat
face moves along its normal onto it (a Move Faces feature), a round face
is offset to the fitted radius (an Offset Faces feature, the log giving
the radius it was likely drawn at and how far the axes lie apart). The
face is never turned, and a scan surface turned more than 20° from it is
refused: a draft or a tilt is a feature of its own.
`pc.design.snap_to_scan{body = …, face_point = …, face_normal = …, scan =
…, at = …}` does the same.

## Mirroring about the scan

Many parts are symmetric: model half and mirror it. **Symmetry plane from
scan** (Design toolbar) finds the plane the visible scan mirrors onto
itself best and keeps it as a datum plane named *Scan symmetry* in the
selected body, refused when its RMS mirror residual is over 0.1 mm. From
then on Mirror mirrors about it by default in that body (a face selected
still wins). `pc.design.scan_symmetry{scan = …, body = …, accept = 0.1}`
does the same and answers the plane and its RMS.

## Fitted surfaces

Faces the solid tools cannot follow are fitted in the Surface bench:
`pc.surface.fit_scan{scan = …, at = {x, y, z}, shape = "plane"}` (or
`"cylinder"`) grows the shape over the scan from `at`, fits it, and adds
it as a sheet that reaches 35 % past the region it was fitted to
(`extend`), so neighbouring fits cross. Each step keeps its fit's RMS
(`sigma`).

**Trim by the scan** (`pc.surface.trim_scan{body = …, scan = …}`) then
cuts the fitted surfaces where they cross each other and keeps each face
that lies on the scan: half its area (`share`) within a tolerance, three
times the loosest fit's σ (at least 0.05 mm) unless `tolerance` is given.
What is kept is sewn, and a closed shell becomes a solid, its faces the
fitted planes and cylinders: fit every side of a scanned block and the
block comes back as a solid to add features to. Free-form regions fitted with a B-spline wait for
the geometry kernel.

For a finished part's full deviation map, tolerances and a report, use
Inspection's [comparison](INSPECT.md#comparing-with-a-reference).

## With an agent

The assistant's **Model from a scan** prompt (its argument is the kind of
part) sets an agent working through the loop this guide describes, and
it reads this guide as `printcad://guide/scan-modelling`:

1. Check the scan (`mesh.inspect`), align it to the axes, find its
   symmetry.
2. Ask once, in one message, what it needs to decide; questions about a
   spot are pinned there as notes.
3. Build as a designer would: sketches on sections, round numbers,
   standard holes and threads read off the scan, half a symmetric part
   mirrored.
4. Hold the model to the scan after each feature with the deviation
   (`design.scan_deviation`), correcting dimensions or snapping faces
   rather than adding shapes.
5. Report each feature as measured, partly supported or inferred, with
   the share of the scan within the tolerance, and say *not done* when the
   target is not met.

The targets by kind of part, never finer than three times the scan's
noise:

| Part | Tolerance | Share of the scan within it |
| --- | --- | --- |
| Machined | 0.02 mm | 90 % |
| Printed | 0.15 mm | 90 % |
| Cast or moulded | 0.5 mm | 85 % |
| Freeform | 0.2 mm | 75 % |
