# Part Design and Sketcher: what is still to build

What the two workbenches do not do yet, grouped by area. Everything not
listed here is built; see the release notes for what is.

## Part Design

### Pad, Pocket and Revolution

- **Up to shape:** stop at a set of faces, not one.
- **Direction:** extrude along a custom vector or a reference edge, not
  only the sketch normal.
- **Two sides, two end conditions:** each side of a two-sided extrusion
  with its own mode (one blind, one up to a face), not two lengths only.
- **Faces as the profile:** pad or pocket a picked planar face of the
  solid, with no sketch.
- **Revolution end modes:** to first, to last and up to face, as the
  extrusions have.
- **Revolution axis:** a reference edge, a datum line or a construction
  line of the sketch, beside the sketch axes and a custom axis.

### Loft, Pipe and Helix

- **Pipe orientation:** fixed and Frenet only; to add a second guide
  path (auxiliary) and a binormal direction.
- **Pipe corners:** how the section turns a sharp corner of the path
  (transformed, right, rounded).
- **Multisection pipe:** several sections along one path.
- **Flat spiral:** a helix of height 0 (growth per turn) is refused
  today.
- **Subtractive helix, keep inside:** keep the intersection instead of
  cutting.

### Hole

- **Thread standards:** only ISO metric today; to add unified inch (UNC,
  UNF, UNEF), British (BSW, BSF), pipe threads (BSP, NPT) with their
  taper.
- **Thread class and hand:** 6H, 2B and the like; left-hand threads.
- **Cuts:** counterdrill, spotface, and standard screw seats; a user
  table of cut profiles.
- **Drill point:** angled (118°, 135°) and whether it counts toward the
  depth; tapered holes.

### Patterns

- **Pattern axis:** a reference edge or datum line; a sketch's own axes.
- **Uneven spacing:** a spacing per occurrence.
- **Polar by step:** an angle between occurrences, beside the overall
  angle.

### Dress-ups

- **Tangent chains:** picking an edge takes the edges tangent to it.
- **Thickness joins:** arc or intersection where the walls meet.

### Datums

- **Attachment modes:** a datum attaches to a base plane or a flat face
  today. To add: tangent to a curved face at a point, through three
  points, normal to an edge, along an edge, through two points, the
  intersection of two planes, a curve's centre of curvature, a shape's
  centre of mass and inertia axes.

### References across bodies

- **Borrowed geometry:** a body using another body's faces, edges or
  sketch (a hole through two bodies, one master sketch driving several),
  live or frozen.

### Generators

- **Involute gear, sprocket and shaft:** profiles made from a few
  numbers, ready to pad.

## Sketcher

### Drawing

- **Conic arcs:** parabolas and hyperbolas.
- **B-spline by points:** a spline through the clicked points
  (interpolation), and a chosen degree; today the clicks are control
  points of a cubic.
- **Rectangle modes:** from three corners, and from the centre and two
  corners; a frame (an offset outline) in one step.
- **Join:** several edges merged into one B-spline.
- **Continuous trim:** trimming everything the pointer drags across.

### Constraints

- **Arc length:** a dimension along an arc.
- **Gap between curves:** a distance between two curves, not only points
  and lines.
- **Radius or diameter by kind:** one tool giving arcs a radius and
  circles a diameter.
- **Angle at a point:** the angle two curves make where they meet.
- **Refraction:** two lines meeting an interface at the angles a ratio
  of indices gives.
- **Auto remove redundants:** a new constraint taking away the older one
  it made redundant, as a preference.

### Editing aids

- **Internal geometry:** an ellipse's axes and foci, a B-spline's control
  polygon, shown and hidden as construction.
- **Intersection references:** another body's geometry cut by the sketch
  plane, beside projected edges.
- **Section view:** everything in front of the sketch plane clipped while
  editing, kept per sketch.
- **Parked constraints:** moving constraint symbols to a second layer to
  declutter.
- **Constraint list filters:** geometric, dimensional, named, reference,
  selected, related to the selection.
- **Remove axis alignment:** turning horizontal and vertical constraints
  into parallel and perpendicular ones so a group rotates as a whole.
