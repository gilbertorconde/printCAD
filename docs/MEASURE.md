# Measure

Reading sizes, distances and angles off the model, and a body's volume,
mass and inertia.

## The measure tool

**Measure** on the toolbar arms the tool; press it again, or Escape, to
put it away. It has no key by default; Preferences › Keyboard can give it
one. While it is armed, clicks in the view pick things to measure instead
of selecting them, and a prompt beside the cursor says what to pick next.
Pan, orbit and zoom work as always ([Camera](CAMERA.md)).

A click picks, in this order:

- the edge under the cursor, when one is within reach
- a datum (a plane, a line, a point or a coordinate system's origin)
- the face under the cursor, the whole face and not just the point
- else the point clicked

Each pick is marked with a cross, the picks are joined by a line, and the
readout stands beside the last two. The log keeps every readout too.

### One pick

| Pick | Reads |
| --- | --- |
| Straight edge | Length |
| Circular edge | Radius, diameter and length |
| Flat face | Area |
| Cylinder or sphere face | Area, radius and diameter |
| Cone face | Area, half angle and cone angle |
| Torus face | Area, major radius, minor radius and tube diameter |
| Datum point or coordinate system | Its name and position |
| Datum plane or line | Its name |
| Point | Its X, Y and Z |

### Two picks

The second pick is measured against the first:

- **Distance** between them. Two parallel flat faces or datum planes read
  the distance square across. A point against a plane reads **Distance to
  plane**, and against an axis **Distance to axis**, both square to it.
  Otherwise the distance is between the points picked, with its Δx, Δy
  and Δz.
- **Least distance** when both picks are faces or edges of bodies: the
  kernel works out the shortest distance between the two, face to face,
  edge to edge or edge to face, and a dashed line joins the closest
  points. It reads "…" while it is being worked out.
- **Axis distance** when both have an axis: a round face, a circular edge
  or a datum line.
- **Angle** when both have a direction: a straight edge's, a circle's or
  a round face's axis, a flat face's normal, a datum plane's normal or a
  datum line's direction.

### Chains

Picks run on in a chain: each new pick is measured against the one before
it, and the chain's totals follow it:

- **Path through N picks**: the length of the line through every pick.
- **Total length** when every pick is an edge.
- **Total area** when every pick is a face.

A click on empty space starts the chain over, as does **Start over** on
the tool's bar under the view toolbar.

### Units

The tool's bar sets the unit its readout is in: **Document** (the
document's own unit) or mm, cm, m, in or ft. The choice is saved with the
document.

## Quick measure in the status bar

The status bar can show what the selection measures, without the tool:
right-click the bar and turn on **Quick measure** (it starts off). Select
faces and edges as usual ([Editing](EDITING.md)) and the bar reads them
as the tool would: one face's area and sizes, the least distance and
angle between the last two, the totals of several. The bar shows the
first lines; hovering it shows them all. It reads in the document's unit.

## The Physical group

The property panel's Physical group measures the body it shows: its
**Volume**, **Surface area** and centre of mass (**Centre X**, **Y**,
**Z**), and its **Mass** once the body has a material. A figure taken
from the body's mesh rather than its exact shape starts with ≈.

Below them is the body's inertia. **Inertia about** chooses what it is
taken about:

- **Centre of mass** (the default) or **Origin**: the inertia tensor
  (Ixx, Iyy, Izz, Ixy, Ixz, Iyz) on the world's axes, then the principal
  moments I1 to I3, smallest first, each with its axis.
- **X axis**, **Y axis**, **Z axis**, an edge picked in the view
  (**Picked edge**, or **Picked edge's axis** for a round one), a round
  face's axis (**Picked face's axis**), or a datum line: the **Moment**
  about that line and the **Gyration radius**, how far from the line the
  whole mass would turn alike.
- A datum point or coordinate system, by name: the tensor and principal
  moments about that point.

With a material the moments are the mass's, in g times the unit squared;
without one they are the volume's at unit density, in the unit to the
fifth.

For a whole assembly, the Assembly's **Mass and centre of mass** tool
weighs every visible body and gives the assembly's principal moments of
inertia about its centre of mass ([Assembly](ASSEMBLY.md)).
