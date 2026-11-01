# RFC 0004: Scans: inspection, mesh editing and reverse engineering

- Status: proposed
- Date: 2026-11-01
- Scope: two new native workbenches (`wb_inspect`, `wb_mesh`), two new
  standalone crates (`metrology`, `image_edges`), the Sketcher, Design
  and Surface benches, `core_document` (scan bodies, marking, mesh
  history), `kernel_api` and `kernel_ogeom` (queries and ops),
  `render_wgpu` (scalar fields, capping, split view), `app_shell`
  (import, marking, maps, reports, agents), `sketch_solver` (soft data),
  `settings`, docs, and a list of kernel requests (§12).

This RFC is a clean-room design. It was written from a functional
description of what scan-inspection and scan-to-CAD software lets a user
do, never from any implementation, and every algorithm named here is the
textbook one. No third-party code, text or assets go into printCAD.

## Summary

A printer owner holds two things printCAD cannot read today: a **scan**
(a dense triangle mesh or point cloud from a structured-light, laser or
photogrammetry scanner, or a flatbed scan of a flat part), and the
question "how far is this from what I meant?". This RFC makes printCAD
answer it, and then use the answer:

1. **Measure a scan** (Inspection workbench): fit planes, cylinders,
   cones, spheres, tori and circles to the scan with their form error,
   construct points, lines and planes from them, dimension between them
   with nominals and tolerances, check form and orientation tolerances,
   align the part by datums, by its own faces, by its symmetry or by
   best fit, cut sections, and report it all.
2. **Compare it**: paint the scan's signed deviation from a printCAD
   body, a STEP file or one fitted element, list the regions of extra
   and missing material, map its wall thickness, animate the
   deformation.
3. **Fix the mesh** (Mesh workbench): islands, holes, smoothing,
   offsetting, cutting, booleans with solids, hollowing, reduction within
   a tolerance, and fitting a replacement piece into a break. Edits are a
   parametric history on the scan body, like a solid's features.
4. **Model from it** (Sketcher, Design, Surface): sketch on a section
   through the scan with its outline fitted into lines and arcs, snap a
   face of a solid onto the scan, read fillet radii, threads and hole
   sizes off it, fit freeform surfaces and patch networks to it, and
   compare the model with the scan while editing.
5. **Measure flat parts from an image** (Sketcher pictures): subpixel
   edges on a calibrated flatbed scan, fitted into sketch geometry, out
   as DXF or SVG at true scale.
6. **Close the loop for printing**: turn the deviation of a printed part
   from its design into the corrections the next print needs (scale per
   axis, hole compensation, first-layer flare, warp), and print test
   coupons that measure them.

Every one of these is a command, so agents and scripts reach all of it
(§9), and every edit is an ordinary op, so undo, collaboration and the
document server need nothing new.

## Motivation

- **A printed part is never the model.** Shrinkage, warp, elephant's
  foot, over-extruded holes and seam bulges are the daily problems of
  FDM and SLA printing. Measuring them today means calipers and guesses.
  A scan measured against the model that made it says where and by how
  much, and printCAD already holds that model.
- **Replacement parts are why many people print.** A broken clip, a
  discontinued knob, a bracket from another machine: the user has the
  part, not the CAD. printCAD imports meshes and can convert them to
  faceted solids, but that is a dead copy, not a model one can change.
  Reverse engineering into parametric features is what makes the part
  printable *and* editable.
- **Repair before printing.** Scans and downloaded meshes come with
  holes, islands and noise. A mesh tool set that is measured (every edit
  compared with the original) is what a printer workflow lacks.
- **Flatbed scanners are the cheapest metrology most makers own.** A
  600 dpi scan resolves 42 µm; traced edges make gaskets, inlays and
  2D parts exact.
- printCAD has the pieces this rests on: mesh bodies, a B-rep kernel
  with sections, fitting and offsets, a measure tool, the Inspect menu,
  a curvature map, sketch reference pictures, PMI reading, body
  placements, a command layer agents drive. What it lacks is the scan as
  a first-class object and the metrology on top.

## Goals

- A scan is a body: it is imported, placed, hidden, saved, collaborated
  on and undone like any other, and large scans (5 M triangles) stay
  interactive.
- Every measurement states what it rests on (points used, cut-off,
  sigma, frame, alignment) and is reproducible from the saved document
  bit for bit.
- Measurements are associative: refitting an element re-evaluates what
  is built on it; moving the scan by an alignment moves everything
  measured on it.
- The new work lives behind the `Workbench` seam: the host never names a
  bench, and the two new benches use only what any bench (or package)
  could use, plus host services added here for all.
- Kernel-side geometry goes to the kernel (§12), never papered over in
  the app; pure statistics and bookkeeping live in a standalone crate.
- Every capability has a command, notes, an example and a recipe.
- Accuracy is tested against closed forms on synthetic scans with a
  known noise model, at the strongest boundary (§11).

## Non-goals

- Driving scanner hardware, or reconstruction from images
  (photogrammetry). A scan arrives as a file.
- Full ISO GD&T in the first version: position, profile, runout, total
  runout, symmetry, material modifiers and multi-datum reference frames
  are staged for a later milestone (§4.6).
- Automatic, unattended reverse engineering as a button. An agent can
  drive the tools end to end (§9.4), but the result is a proposal the
  user reviews.
- Texture-mapped rendering of scans beyond per-vertex colour.

## Background: what printCAD has today

| Piece | Where | Used by this RFC for |
| --- | --- | --- |
| Mesh bodies (STL, OBJ, 3MF, PLY, glTF, VRML), welded, outlined at creases | `kernel_ogeom/src/mesh.rs`, `Document::is_mesh_body` | The scan body (§2.1) |
| Convert to solid, refine faceted shapes | `drive_mesh_solids`, `refine_solid` | Mesh-to-solid after editing; analytic recognition |
| Body placement, local and placed geometry | `core_document/src/placement.rs` | Alignment is a placement (§4.5) |
| Measure tool, quick measure, Physical group | `app/measure.rs` | Two-pick readouts extended to scan elements |
| Inspect menu (`INSPECT_CATEGORY`) | `ui/menu_bar.rs` | The new checks file there |
| Curvature map, zebra stripes | `wb_surface/src/analysis.rs` | Pattern for scalar maps; mesh curvature (§5.4) |
| Overlay meshes with vertex colours, `on_top` pass | `render_wgpu`, `OverlayMesh` | Maps until the scalar pass lands (§7.2) |
| Clip plane, section through the view | `camera/section.rs` | Section view; capping added (§7.4) |
| Sketch reference pictures, two-point calibration | `wb_sketch/src/images.rs` | 2D image measurement (§6.3) |
| Sketch external geometry from solids, sections of faces | `wb_sketch/src/external.rs`, `KernelQueries::section_face` | Scan sections in sketches (§6.1) |
| Wall-thickness check of a sketch profile | `wb_sketch/src/walls.rs`, `KernelQueries::medial_axis` | 2D only; 3D thickness is new (§5.3) |
| Design Move/Offset/Delete Faces, Hole tables, threads | `wb_design` | Snap to scan, read holes and threads (§6.2) |
| Surface fill, loft, sew, trim, curvature | `wb_surface` | Fitted surfaces and patch networks (§6.4) |
| PMI read from STEP and IGES | `kernel_ogeom/src/annotations.rs` | Nominals and tolerances from CAD (§4.4) |
| Commands, recorder, MCP, `view` pictures | `core_document::command`, `app/mcp.rs`, `proof.rs` | Agents (§9) |

The kernel survey for this RFC (ogeom 0.12) found: plane, sphere,
cylinder, cone and torus *recognition* within a tolerance
(`algo::recognize_points`), constrained region fits inside
mesh-to-solid (`MeshRegions::fit`), B-spline curve fitting
(`geom::fit::fit_points*`) and scattered height-field surface fitting
(`fit_surface_scattered`), point-to-surface projection, exact
silhouettes and hidden-line projection (`hlr`), 2D wire offsets for lines
and arcs (`offset::offset_wire`), quadric mesh decimation
(`mesh::simplify`), semantic PMI writing, and none of: plain best fits
with residuals, registration, point-to-mesh distance, mesh ray casting,
a public spatial index, mesh topology editing, mesh booleans, 3D
thickness, binary PLY or point clouds, or wireframe STEP. §12 lists what
to ask for.

## Design

### 0. Where each piece lives

| Capability | Lives in | Why there |
| --- | --- | --- |
| Scan bodies, point clouds, colours, units, large-mesh storage | `core_document`, `kernel_ogeom` import, `app_shell` | Bodies are the host's |
| Surface marking (brush, window, lasso) and marked sets | host service in `core_document::runtime` + `app_shell` | Inspection, Mesh, Design and Surface all mark |
| Scalar maps (deviation, thickness, curvature, accuracy) | `render_wgpu` scalar pass, `app_shell` legend/histogram | One renderer path for every map |
| Fitting, constructions, dimensions, GD&T, alignment, sections, reports | new bench `wb_inspect` | A coherent tool set with its own feature kinds |
| Mesh editing as history | new bench `wb_mesh` | Mesh features on scan bodies |
| Statistics, GD&T evaluation, colour ramps, report model, design-value rounding | new crate `metrology` (std-only, copyable, like `sketch_solver`) | Pure, testable, shared by both benches and the host |
| Image edge detection, subpixel chains | new crate `image_edges` (std-only) | Pure image maths |
| Sketch on scan sections, chain fitting, image measurement | `wb_sketch` | They are sketches |
| Snap face to scan, extract element as body, read holes/threads/rounds | `wb_design` | They are Design features |
| Fitted surfaces, patch networks, trim-to-scan | `wb_surface` | They are surface steps |
| Geometry (fits, registration, distances, rays, mesh ops, thickness, volumes) | the kernel, through `KernelQueries` and `SolidOp` | Kernel gap protocol |
| Print compensation | `wb_inspect` (measure) + host print settings (apply) | Measurement there, export here |

Layering: `metrology` and `image_edges` sit with `sketch_solver` at the
bottom; `wb_inspect` and `wb_mesh` sit beside `wb_design` and
`wb_surface` (they read sketches and solids through the document and
the kernel, never a bench crate); `scripts/check-layers.mjs` gains both.

### 1. Scans in the document

#### 1.1 The scan body

A scan is a mesh body (`Body` with `ImportedGeometry` and no shape
snapshot) carrying a new `scan: Option<ScanInfo>`:

- `source`: the file as an asset (bytes kept exactly as opened, as an
  import's are), its format and the units it was read in.
- `kind`: `Mesh` or `PointCloud` (vertices with normals, no triangles).
- `colours`: per-vertex sRGB when the file has them (PLY colour, OBJ +
  MTL + texture sampled per vertex, 3MF colour groups and textures).
- `noise`: the scan's measured noise (RMS of a local quadric fit over
  a sample), derived and cached, which sets every default that depends
  on it (fit cut-offs, smoothing size, tolerance floors).
- `spacing`: median edge length, derived.
- The mesh lives as an entry of its own (`mesh/<uuid>.bin`, as today),
  with indexed vertices kept (not de-indexed) so vertex ids are stable
  for markings and edited copies.

Op `SetScanInfo` (a history barrier, as an import is) marks a mesh body
as a scan; File › Import offers "as a scan" for mesh formats, and
`doc.import_scan` does the same. Any mesh body can be promoted later
("Use as scan" in the body menu).

A scan body takes no Design features but takes Mesh features (§3) and
is the *subject* of Inspection features (§4). Its tree row shows a
SCAN badge, the triangle count, the noise and the units it was read in.

#### 1.2 Import

- **Formats**: STL (binary, ASCII), PLY (ASCII and **binary**, with
  colour and normals; a header comment `printcad keep-winding` skips
  orientation repair), OBJ (with MTL and texture), 3MF (build items,
  components, mirrored instances, colour groups, base materials and
  textures; a slicer project's modifier volumes, support blockers and
  enforcers and negative volumes are left out), STEP/IGES (tessellated
  finer than for display: edges no longer than 1/250 of the diagonal),
  XYZ and PTS point clouds, binary and ASCII.
- **Units**: STL asks mm, cm, m or inch (the last answer first, "do not
  ask again" in Preferences › Files, with the default it then uses);
  PLY and OBJ default to mm with the same question available; 3MF, STEP
  and glTF carry their own. The answer is stored on the scan.
- **Orientation**: a closed mesh enclosing negative volume is flipped
  outward; an open one keeps its winding.
- **Safety**: nesting and size caps (one million objects, 30 million
  vertices) refuse a file that would exhaust memory, as the project
  reader's caps do.
- **Large files** import on the kernel worker with progress, as STEP
  does today; the mesh is welded at a tolerance from its own spacing.

#### 1.3 Export

- Scan mesh as STL (mm or inch), 3MF (mm, colours kept), PLY, in the
  pose it is placed at; never under the source file's name (suffixes
  `-aligned`, `-edited`, `-reduced`).
- Point cloud as binary PLY or XYZ text (`x y z nx ny nz`), normals
  outward.
- File › Export already takes bodies; scans join it with these options.

#### 1.4 Large scans

A 5 M-triangle scan must orbit at interactive rates and pick in one
frame:

- **Renderer**: index buffers of 32-bit indices already work; add a
  coarse level of detail (a decimated copy at 1/16, made on the worker)
  drawn while the camera moves, the full mesh when still, as solids are
  meshed coarse while moving.
- **Picking**: the GPU pick returns the triangle (`PickResult::triangle`
  exists); hover readouts of maps interpolate across it from the
  barycentric coordinates the pick pass also writes (new: two more
  channels).
- **Queries**: every scan query goes through a kernel spatial index
  built once per scan revision on the worker (K4) and kept beside the
  mesh.
- **Memory**: scalar fields are `f32` per vertex in GPU buffers, never
  copied into the mesh.

#### 1.5 Marking: the host's surface selection

Inspection (hand-marked fits, local fine fit, element maps), Mesh
(delete, flip, smooth, offset), Design (fit regions) and Surface
(fitted-surface regions) all mark scan surface. The host owns it:

- `WorkbenchRuntimeContext::marking`: the active marking (a body, a
  bitset over its vertices, and its colour role), and
  `ctx.request(HostRequest::Mark(MarkTool))` arms a tool: `Navigate`,
  `Window`, `Brush { diameter_mm }`, `Lasso`, with `erase`,
  `through` ("include faces pointing away") and `invert` options.
- Gestures: left drag marks, right drag or Alt erases, Shift drag
  orbits, the middle button stays the camera's. The brush shows its
  footprint ring on the surface under the cursor.
- A gesture takes whole triangles it covers; by default only
  front-facing ones (a window never marks the far wall); invert flips
  per vertex and leaves the mixed border row bare both ways.
- Escape backs out one level: the gesture, then the tool (keeping the
  marking), then the marking (which waits as "Restore" until the next
  tool starts).
- The marking is a document object only when a feature keeps it: a
  feature that uses a marking stores it as an asset (a compressed vertex
  bitset keyed by the scan's revision) so it survives save and undo.
- Region growing from a click (§4.1) produces the same bitset, so a
  grown region can be edited by hand.

### 2. Units, display and interaction for measurement

- **Inch display**: Preferences › General › Units, mm or inch, for
  display and typing only (one more decimal in inches, at least three;
  areas and volumes in in² and in³). Storage, files, commands and agent
  answers stay mm; reports say "lengths in inches". This is new for the
  whole app, not only scans.
- **Constant-size markers**: pick pins, grips, rings and note badges
  keep their screen size at any zoom (the gizmo already does).
- **Numbered pick pins**: each click of an element in progress is
  pinned and numbered until the element is made.
- **Universal confirm**: Enter or a still middle click runs the task's
  primary button in every task (the task panel already maps Enter; the
  middle click is added).
- **Hide pins behind the part**: labels and pins test depth against the
  scene's depth buffer (read back with picks) and fade when occluded.
- **Colour modes**: a "scanner blue" glossy material for reading surface
  quality, beside the default; measurement colours never change with it.
- **Sharp-edge shading**: per body, always sharp, auto (sharp when the
  mesh reads as a tessellation), or smooth, with a crease angle
  (default 30°).

### 3. The Mesh workbench (`wb.mesh`)

Mesh editing is a **history of mesh features on the scan body**, built
by the kernel worker exactly as a solid's history is: a `MeshPlan` of
`MeshOp`s per body, rebuilt when an earlier feature changes, failures
attributed to the feature that failed. The tip is the mesh every other
bench measures; moving the tip back ("use the original") is the
existing tree navigation. This replaces the "edited copy" idea with the
mechanism printCAD already has: every edit is undoable, reorderable,
editable and saved, and the original is the history's start.

`kernel_api` gains `MeshOp` (a new contract enum; packages may build it
later under the same additive rules as `SolidOp`) and
`KernelQueries::build_mesh_chain`. Each op carries its region as a
marking asset where it needs one.

| Feature | Parameters | Result | Kernel |
| --- | --- | --- | --- |
| Remove islands | "smaller than" (default 1/100 of the diagonal, bounding-box size) | Pieces smaller than that removed; preview tints islands and rings them through the part, counts pieces, largest island, smallest kept | K10 components |
| Delete marked | marking | Triangles with all three corners marked removed; the hole stays open | K10 |
| Flip normals | all, or marking | Winding swapped (vertices unchanged) | trivial, app side |
| Cut by plane | plane (XY/YZ/XZ in the current frame, or an element's direction), position, side kept, "close flat" (default on) | Triangles clipped at the plane (one shared vertex per crossing edge, near vertices snapped), every closed loop capped with nesting, rim gaps up to 1/10 of the rim bridged, the rest left open and counted; live preview with the actual cap | K11 plane cut + cap |
| Fill holes | "holes up to" (default 1/20 of the diagonal, longest span), all or one by clicking its ring, fairing Curved (bi-Laplacian including one ring of the scan) or Flat | Triangulation minimising the largest dihedral angle then area (exact dynamic programming up to 500 rim edges, planar ear clipping beyond), refined to the surrounding density; scan vertices never move | K12 |
| Smooth | whole or marking, "bumps up to" (default 1/125 of the diagonal, never under 2.5 edge lengths), "move at most" (1/1000), keep sharp edges (30°) | Each point projected onto a local weighted quadric over ~100 neighbours, twice; marked regions blend to zero at their edge; status reports max and RMS movement | K13 |
| Offset (shrink/expand) | whole or marking, distance (default 0.1 mm) | A true offset, not a scale: each vertex placed so its faces end at the distance (corners √2, √3), folds repaired; a marked region moves as a step with a joining wall | K14 |
| Boolean with solids | subtract, add, intersect; operands: Inspection elements as drawn (cylinders, cones, spheres, with assumed sizes and extensions) and any solid body | Exact mesh boolean on a closed mesh; refuses an open one ("fill holes first") and operands that miss | K15 |
| Shell / hollow | closed part: hollow (inner wall at a thickness, cavity sealed); open surface: thicken inward, outward or both with rim walls; thickness default 1/125 | Distance-field hollowing on a grid of a third of the wall, marching tetrahedra, decimated within 1/16 cell | K16 |
| Reduce | tolerance (default ≈ diagonal/4000 on 1-2-5 steps), triangle limit (default 50 000) | Quadric edge collapse verified both ways at 99.9 %, offending points pinned and repeated until it holds; reports share within, 99.9 % distance, furthest point, RMS; Accuracy map | K17 (extends `mesh::simplify`) |
| Trim to scan (repair piece) | a solid body modelled oversized into a break, break depth (0.3 mm), offset from break (glue gap), side | The break (scan deeper inside the body than the depth) extended and thickened into a cutter; the body cut by it; readout of piece volume, mating area, gap median and 90 % range; Gap/Piece/Break views | K18 |
| Make solid | — | The existing Convert to solid, now available on any history tip | exists |

The bench's panel lists the history with each step's triangle counts
and a **Deviation of the edit** view: the tip coloured by signed distance
to the history's start (added surface, such as fills, caps and boolean
walls, grey), scale at the 99th percentile, with max, RMS and the share
of added area. "Use for analysis" is not a separate state: Inspection
measures the tip, and its features refit when the tip changes (§4.9).

Commands: `mesh.islands`, `mesh.delete`, `mesh.flip`, `mesh.cut`,
`mesh.fill`, `mesh.smooth`, `mesh.offset`, `mesh.boolean`,
`mesh.shell`, `mesh.reduce`, `mesh.trim_to_scan`, `mesh.inspect`
(triangles, bounds, open edges, volume, islands and holes at a size
with their centres), `mesh.preview_cut`, `mesh.export`. Regions in
commands are a marking, a box, a ball, a vertex list, each filtered by
facing.

### 4. The Inspection workbench (`wb.inspect`)

Its features live in an **Inspection** group under the scan body in the
tree (feature kind `wb.inspect`, one node per element, dimension,
tolerance, section, alignment and map), stored in the scan body's own
frame, so a placement (alignment) carries them all. Tool categories
(and so tool panels and the radial menu): Fit, Construct, Dimension,
Tolerance, Align, Section, Compare, Thickness, Report. The Compare,
Thickness and Section-view tools also carry `INSPECT_CATEGORY` so the
Inspect menu lists them from any bench.

#### 4.1 Fitted elements

**Kinds**: plane, sphere, cylinder, cone, torus, circle (3D, on a
plane), line (3D, from a narrow strip or an edge), point (picked).

**Choosing the surface**:

- *Click to fit*: press the kind, click the feature. The region grows
  from the click over the mesh with the model of the kind being fitted
  (refit as it grows, normals checked against the model), so a sphere
  stops at its rod, a bore at its end faces, a plane at its edges. More
  clicks add patches of a broken feature; each seed reports whether it
  landed in the region and its offset and angle from the fit, and a seed
  on another surface warns "fit these separately". "Undo point" removes
  the last.
- *Edge peeling*: after growing, the region's rim is peeled ring by ring
  while its normals turn away from the model by clearly more than the
  noise, so a scanner's rounded edges do not pull a face or flare a bore.
- *Hand-marked*: "Surface: marked by hand" fits exactly the marked
  triangles (§1.5).
- *Guided*: side (inside of a bore, outside of a boss), radius range,
  approximate axis, box, and for spheres a **known diameter** (fit the
  centre only; refused if it differs from the free fit by more than 1 mm
  or 10 %), reported with how far holding it moved the centre.

**Fit**: an orthogonal (Gaussian) least-squares fit started from a
robust estimate (consensus on samples), with an outlier cut-off of all,
3σ (default), 2σ or 1σ per element (the last choice seeds the next).
Kernel K1. Later: minimum-zone, maximum-inscribed and
minimum-circumscribed fits as alternative methods per element (K2).

**Results** (every element): size (radius or diameter, cone half-angle
and apex, torus ring and tube radii), position and direction, sigma
(RMS of used points from the ideal shape), form error (peak-to-peak),
points used of points in the region, and for spheres, cylinders and
circles Ø min and Ø max (the Gaussian radius moved to the extreme
residuals, doubled). Cylinders report fitted length and the arc of wall
covered; planes their patch size.

**Diagnostics**, each a warning on the element with numbers:

- a stepped bore (region split along the axis: the seed's part fitted,
  the other's radius, position and share reported);
- a drafted cylinder (draft read from the normals; a wall drafted both
  ways from a parting line named as two cones with the line);
- a region spanning two surfaces (share, axial band, axis angle, radii
  of each);
- a plane on the crown of a bead (normals spread over 10° and a quadric
  rising over 3× the residual) or filling under 15 % of its box;
- a plane spanning two facets (where the second is, angle, offset);
- a flat patch offered as a cylinder (refused, larger patch tried);
- a bore bridged by the scanner (axis running into a closed skin).

**Assumed (design) size**: an optional "assumed Ø" for spheres,
cylinders and circles, flagged as a probable typo beyond 0.5 mm (or 5 %
on large features). Readouts, dimensions and maps keep the measured
value; drawing, STEP export and bodies made from the element use the
assumed one; reports show both.

**Designed relation**: hold a plane, line, cylinder, cone, circle or
torus parallel or perpendicular to a reference (a coordinate plane of
the current frame, a measured plane, a section): the element turns
about its centre by the least rotation, the panel shows how far off the
measurement was and warns past 2°. Sigma and form stay the
measurement's.

**Extents**: cylinders take two end extensions, planes four edge
extensions (±U, ±V), with grips on the part and "make square"; display
and export only, except the cylinder's "fit only inside span", which
refits on the scan between the drawn ends.

**Edit**: reopening restores the pins and the live fit; more picks,
undo point, a change of method (fitted plane to plane through three
points) or kind keep the element's id, colour and place; what is built
on it re-evaluates.

#### 4.2 Constructed elements

Associative constructions that follow their sources and are deleted
with them, cycles refused:

- **Point**: picked on the scan; typed; centroid of the scan (of its
  enclosed volume, or of the surface when open, or of a marking);
  midpoint of two points; a line or axis through a plane.
- **Line**: through two points; a cylinder's or cone's axis; two planes'
  intersection.
- **Plane**: through three points; offset from a plane; midplane of two;
  typed normal and point; **symmetry plane of the scan** (reflect a
  sample through a candidate plane, register the reflection back onto
  the scan, bisect each sample and its registered image, repeat until
  stable; seeds from the principal planes and the faces auto-align
  finds, or a user seed; on a marking if one is given; sigma is the
  mirror residual, so an asymmetric part reports a loose fit). Kernel K5.
- **Circle**: through three or more points; plane through cylinder
  (refused beyond 5° of lean, where the cut is an ellipse); plane through
  sphere; typed.
- **Substitution**: wherever a point is asked, a sphere or circle gives
  its centre; an axis, a cylinder, cone or circle gives its axis; a
  plane, a section gives its cutting plane.
- Point-only constructions prompt for their slots in turn; a click on
  the scan makes a picked point, on an element uses its centre.

#### 4.3 Dimensions

New dimension, choose the type, pick two elements (viewport clicks fill
the slots); the box stays open for the next until Escape.

| Type | Reads |
| --- | --- |
| Point–point | Centre distance with signed ΔX, ΔY, ΔZ in the current frame; between spheres also the surface gap or the outer span |
| Point–axis | Perpendicular distance |
| Point–plane | Signed distance along the plane's outward normal |
| Axis–axis | Offset of near-parallel axes, or distance at closest approach |
| Axis–plane, plane–plane | Signed distance from the middle of the first; plus the surface range (min and max once each face's form is included: what a caliper finds) |
| Angle axis–axis, axis–plane | 0–90° |
| Angle plane–plane | Between outward normals, 0–180° |
| Diameter / radius | Of a sphere, cylinder, circle, with Ø min/max |

Rules: fitted planes are only their measured patch and axes only their
fitted length, so a dimension that extrapolates warns; planes more than
3° from parallel get no distance (use an angle). Each dimension takes a
**nominal** and a **± tolerance** (asymmetric allowed); the row and the
label on the part show the deviation and turn red out of band; reports
say PASS or FAIL and tally. Labels on the part spread apart so they do
not overlap.

An in-app note (and the guide) explains why a hand slice reads plane
angles low and how to check one properly: construct the intersection
line, section square to it, fit lines in 2D.

#### 4.4 Nominals from the model

When the scan is compared with a printCAD body or a STEP part (§5.1),
dimensions can take their nominal from the model: pick the same feature
on the model (a face, a bore) and the nominal is read from its exact
geometry; PMI read from a STEP file (already imported as annotations)
offers its dimensions and tolerances as nominals in one click.

#### 4.5 Alignment

Alignment moves the **scan** (its body placement), never the reference;
everything measured on the scan is in the scan's own frame, so it comes
along. Each alignment is an Inspection feature recording how it was
made, and applying it records one `SetBodyPlacement`; "Reset alignment"
returns to the scanner's frame in one undoable step.

- **3-2-1 datums**: (1) set on a plane: a measured plane or cylinder, or
  three picks on a face, and which side of the part it is (bottom, front
  …); (2) direction: a plane normal, an axis or two picks along an edge,
  and which way it runs; (3) zero: a point, a sphere's or circle's
  centre, or a pick. Each step zeroes what it fixes; the first alignment
  centres the part along any free coordinate (optional). A hint when a
  parallel face within 5 mm holds more scan. Live preview: the part
  swings to its would-be pose as soon as a step is satisfiable.
- **Auto-align**: proposes the three steps from the scan alone: face
  directions from an area-weighted histogram of smoothed normals
  (antipodes merged), a turned or extruded part's axis from walls
  square to it, planar open rims voting as the face that would close
  them, the largest flat face down, the long side along X, the zero on
  the standing face or the common axis of round walls; refined by the
  closed-form best rotation on the normals; it says what it rests on and
  how square the part is, or that it fell back to principal axes. The
  proposal is held as picks, so the side and direction choices give all
  24 poses. Kernel K6.
- **Use symmetry**: turns the axis nearest the symmetry plane's normal
  onto it, re-squares the others with least change, drops the zero onto
  the plane; refuses beyond 0.5 mm mirror residual.
- **Move and turn by numbers**: typed translation and rotation (X, then
  Y, then Z about the zero).
- **Best fit to a reference** (§5.1): rigid only; point-to-plane ICP
  with an adaptive outlier cut-off (a multiple of the median), started
  from the identity and the 24 principal-axis matchings, candidates
  scored by the capped mean distance of all samples (not only inliers);
  passes stream to the view and Escape restores the previous pose; warns
  when the fit slides along one surface. Kernel K3.
- **Align by picked points**: a split view (scan | reference, §7.5),
  alternate clicks on corresponding features, three pairs suffice:
  closed-form absolute orientation, then ICP, near-collinear picks
  refused; a marking on the scan half limits what the fit may settle on.
- **Local fine fit**: from the current pose on a marked subset; marked
  points pair only with reference surface facing within 60°, within a
  hard search distance (1 mm); a selection facing one way is flagged as
  free to slide; "back to the global fit" restores exactly.

#### 4.6 Form and orientation tolerances (GD&T)

New tolerance: characteristic, toleranced element, datum (any element,
named as in the list), limit. The characteristic follows the picks.

- First version: flatness, cylindricity, sphericity, circularity,
  straightness (line), parallelism, perpendicularity, angularity (at a
  basic angle), coaxiality, concentricity. A plane is its measured
  surface (orientation includes its flatness); cylinders, cones and
  lines are their axes over the fitted length; spheres and circles
  their centres; a constructed plane is evaluated from its drawn
  corners, with a warning. Evaluation lives in `metrology` over the
  element's used points.
- Second version: position (with a datum reference frame of up to three
  datums and basic dimensions), profile of a surface and of a line
  against the reference model, circular and total runout, symmetry,
  MMC/LMC modifiers and bonus tolerance, minimum-zone evaluation of
  form (K2).
- Export: tolerances and datums are written as semantic PMI on a STEP
  export of the reference or of a model built from the scan (the kernel
  writes `Pmi` already; the app wires it). Kernel K22 for what remains.

#### 4.7 Sections

- **Cut**: a plane across an element's direction or a coordinate plane
  (offered as translucent sheets in axis colours through the part
  centre), at a typed offset or dragged with the transform gizmo (slide
  and tilt); a tilted plane records its tilt and "Square" removes it;
  clicking the scan moves the plane there. Chains shorter than 1 mm are
  dropped. The plane is frozen at creation (refitting its source does
  not move it); alignment carries it. Kernel K7 (mesh section into
  polylines carrying the out-of-material side).
- **Through an axis**: a half-section containing an axis, at an angle
  about it (radial distance × axial position).
- **Shadow**: the whole scan projected along the normal, outer boundary
  only (holes kept), fitted as lines meeting at corners, arcs and full
  circles to a tolerance (default 0.1 mm) and guaranteed outside the
  part by a play (default 1 mm, 0 exact, negative a press fit): every
  piece between play and play + 2 × tolerance outside the silhouette;
  corners turning less than "round below" (30°) take the largest tangent
  arc that fits; spikes removed. Readout: loops, area, band achieved.
  "Extrude to part thickness" (default on) stands it up as a block; a
  STEP export writes one closed solid per outer loop. This is the bin,
  foam-insert and pocket generator printers ask for. Kernel K8.
- **Stack**: several offsets of one plane at once, each fitted; chains
  tracked across heights for tilt, draft, taper and round radius.
- Every section is a **sketch source** (§6.1): opening a sketch on it
  measures the cut in 2D with the Sketcher's tools.

#### 4.8 Reports

- **Copy summary / report**: every element, dimension and tolerance
  with what it rests on (units line, frame and alignment, each fit's
  cut-off, residual extremes, Ø min/max, surface ranges, assumed vs
  measured, extensions drawn vs measured, designed relations and their
  deviation, symmetry search counts), PASS/FAIL per row and a final
  tally; as text, CSV or JSON (`inspect.report`).
- **Report document**: a printable page (PDF through the CPU picture
  renderer plus a text layout): pictures from fixed views with labels,
  the table, the maps' legends and statistics, the traceability header.
  Later: templates.
- **Elements as STEP**: planes as bounded faces, cylinders and spheres
  as solids, lines and points as wireframe, each a named shape; or all
  as construction surfaces and curves in one geometric set; assumed
  sizes applied, extents as drawn; sections' fitted 2D geometry as
  curves in their planes. Kernel K21 (wireframe write).

#### 4.9 Associativity and rebuild

Fits are **derived state** the bench works out from the scan's tip, the
element's region definition (seeds and settings, or a marking asset)
and its method, cached by those inputs; `derive_on_geometry` refits when
the scan's tip changes (a mesh edit), and constructions, dimensions and
tolerances follow. A fit saved in a document reads back identically:
fitting is deterministic (fixed sample order, fixed seeds), and a
corpus test holds saved results to their measures (§11). Region growing
reruns from the seeds on a changed mesh; a hand marking keeps the
vertices that survive.

### 5. Comparing (deviation, volume, thickness)

#### 5.1 Deviation from a reference

**Reference**: any body in the document (a solid built in Design, a
STEP import, another mesh), or a file opened as a reference (STEP,
IGES, mesh). A B-rep reference is meshed by curvature only (a flat face
stays two triangles) at a chord scaled to the part (0.01 mm on 100 mm),
stated as the measurement floor; a reference whose solid has cracks is
reported as unreliable for inside and outside. Kernel K9 for the
curvature-only tessellation setting.

**Map**: the signed distance from each scan vertex to the nearest point
of the reference, positive where the scan lies outside; the sign from
the angle-weighted pseudonormal of the nearest feature (no false flips
at seams); facing-aware (when the nearest reference surface faces away
from the scan point, the nearest one facing its way is used; option
"max deviation of normals", default 90°); grey beyond a search distance
or with nothing facing within 1/10 of the diagonal; inside-out scans
detected. Kernel K3 (point-to-mesh/B-rep signed distance, batched).

**Display** (a host scalar map, §7.2): range ± (default the rounded
95th percentile, capped at 1 mm), continuous or banded, a colour ramp
with zero at its centre and distinct end caps (jet by default and eight
perceptually uniform, colour-blind-safe ramps), a histogram sharing the
legend's axis with min, max, mean, RMS and sigma, a tolerance ± that
counts the share within, a search distance that greys and excludes.
Hover reads the deviation under the cursor; a click pins a reading on
the part; pins hide behind the part.

**On the reference**: the reverse map, the reference painted by distance
to the scan (grey where no scan lies behind it), with coverage shares.

**Hot spots**: vertices beyond tolerance grouped by proximity on the
same side, largest first, each with centre, box, area, count, mean and
extreme, and classified *feature* or *texture* by size and depth.

**Volume errors**: compare scan and reference as solids on a grid:
regions of missing and excess material with volume and depth, each a
layer (a face standing off) or a lump, including behind walls; scan
holes filled from the reference; unsure regions marked. Kernel K19.

**Deformation animation**: the map played as exaggerated motion (ideal →
measured × scale → back), each point moving along its measurement
direction, motion smoothed over ~1 % of the diagonal so warps move and
noise does not; GPU only (§7.2).

**From one element**: the map against a fitted plane, cylinder or sphere
instead of a reference: no alignment needed, within the element's drawn
extent, material side from the scan's normals (Flip), a facing filter,
on a marking. "Is this face flat, is this bore round?"

**Export**: signed distance per vertex as binary PLY scalar, CSV or a
NumPy array (`inspect.map_export`); height maps of deviation over a
plane.

#### 5.2 Wall thickness

No reference, no alignment: **ray** (from each vertex along the inward
normal to the exit, with an optional cone of rays, shortest kept) or
**sphere** (a sphere centred halfway along the ray grown until it
touches, never longer than the ray). Both triangle sides count as exits;
surfaces nearly edge-on to the ray are skipped (max deviation of normals,
default 60°); nothing within the search limit (default a fifth of the
diagonal) is grey. Ramp reversed (thin hot), defaults at the 2nd and
95th percentiles, histogram, "thinner than" share. Kernel K20 (batched
ray casting against a mesh or B-rep, nearest-point queries).

This is printCAD's first 3D wall-thickness check, and it runs on
**solids too**: a printability check of a Design body against the
printer's minimum wall (Printing preferences), offered in the Inspect
menu for any body.

#### 5.3 Curvature on scans

Mean curvature on the scan's vertices from local quadric fits, averaged
over neighbourhoods so noise does not dominate, scale at the 90th
percentile read as an equivalent cylinder radius; the Surface bench's
curvature map gains mesh bodies. Kernel K13 (shares the local quadric).

### 6. Reverse engineering in the existing benches

#### 6.1 Sketcher: sketching on a scan

- **Plane choice**: a section, a fitted plane, a coordinate plane (at
  zero once aligned, or through the scan's centre), at a typed offset or
  a formula; the first sketch on an unaligned scan warns that the model
  will be built in scanner coordinates and links to Align.
- **Scan section as guide**: the slice of every visible scan body by the
  sketch plane becomes guide geometry (`ExternalSource::ScanSection`),
  drawn as point chains, followed when the scan's tip changes; the part
  is cut open at the plane (capped, §7.4) with "No part / Section /
  Full part" under the sketch.
- **Fit all chains**: every chain becomes lines and arcs with corners
  joined, closed when the slice closes; a bow within several times the
  noise stays one line; blobs in corners (spatter, support remains)
  give way to the sharp corner; small step risers are kept. Options:
  detail, tolerance, join corners, align lines to the sheet axes within
  a few degrees (as removable inferred constraints), merge collinear
  pieces; a change refits as one undo step. Infer corners (ends within
  2 mm joined, tangency where a line meets an arc) and Close (extend two
  lines to their intersection, else bridge). Kernel K23 (fit a chain of
  lines and arcs to a polyline within a tolerance); the bench keeps the
  solver side.
- **Soft data in the solver**: fitted pieces keep their slice points as
  *soft* data: constraints and dimensions are met exactly, the points
  settle the remaining freedom by least squares, unachievable dimensions
  are reported, not compromised. `sketch_solver` gains weighted soft
  residuals (a second objective in its LM solve, after the hard
  constraints are satisfied), its own RFC 0003 contract extended.
- **Fit spline to slice**: a selected spline takes nearby slice points
  as soft data.
- **Design-value suggestions**: every measured length, radius and angle
  shows a chip with its probable design value (halves under 10 mm, whole
  millimetres above, radii to 0.5 mm, angles to 5°; inch values in inch
  mode); one click makes it a driving dimension. Candidate relations
  (level, upright, parallel, perpendicular, collinear, equal, concentric,
  tangent, symmetric, on a coordinate plane) are offered as evidence
  with their offset in sigma, never applied silently (`sketch.candidates`).
- **Symmetric relation by three picks** (two pieces, then the mirror
  line), and a sketch pattern (linear, grid, circular) of selected
  pieces with their internal constraints, if not already present.
- **Live deviation**: while the sketch is open, the slice points are
  coloured by their distance to the sketch geometry within a range
  (default 0.1 mm); a count of points within range.

#### 6.2 Design: features from the scan

- **Live deviation while editing**: with a feature's task open, scan
  near the preview is coloured by distance to the preview solid (green
  on, red outside, blue inside) within a range (default 0.1 mm), farther
  scan left plain; the task shows points within range. After building,
  a **Deviation** view of the body against the scan paints scan lying
  deeper inside the body than the reach in violet (a missing hole or
  pocket).
- **Snap to scan**: pick a body face, then click where it should lie on
  the scan: a plane or cylinder is fitted there and the face moves onto
  it (flat faces along their normal, round faces change radius), the
  task showing fit σ, angle between faces, axis offset, a roundable
  value; scan clicks more than 20° off are refused. It creates a Move
  Faces feature, or, when traceable, **changes what made the face**
  (the pad's length, the sketch line or circle behind a wall, a
  revolution's profile line), with the reason when it cannot. If the
  scan face leans beyond what its scatter explains, it offers "draft it
  as well, by N°".
- **Extract element as body**: a fitted plane (as a slab), cylinder,
  cone, sphere or torus becomes a Primitive at its fitted extents and
  assumed size.
- **Read off the scan**: a fillet's radius across the first edge
  (section square to the edge, circle tangent to both faces); a hole's
  diameter and depth from a fitted cylinder; a thread's pitch, major and
  minor diameter, hand and nearest standard (the Hole tables already
  hold the standards) along a cylinder (`design.read_thread`).
- **Mirror about the scan's symmetry plane** by default when one exists
  or can be found (accepted at ≤ 0.1 mm RMS over half the samples).
- **Cut by sheet, keep what lies on the scan**: split a body by sheets
  and keep the fragments that lie on the scan (area-sampled within a
  tolerance and an overlap share).
- **Pick fingerprints** already exist (naming); nothing new.

#### 6.3 Sketcher: measuring images (flatbed scans)

The Sketcher's reference pictures become a measurement surface:

- **Scale states**: none (pixels), nominal (the file's dpi: PNG pHYs,
  JPEG density) with a persistent UNCALIBRATED mark, calibrated by a
  known distance (two picks), a known circle (three or more picks), or
  separately in X and Y (sensor and transport axes). **Scanner
  profiles** (named calibrations per scanner and resolution) live in
  user settings and apply to later images.
- **Edges**: subpixel edge chains detected once on load on a thread
  (gradient detector with automatic thresholds, chains linked, each
  point refined by a parabola across the gradient, small breaks bridged,
  chains under 1 mm dropped, one side brighter along a chain, one
  sensitivity control), drawn over the picture as guide geometry and kept
  in pixel coordinates so recalibration re-derives everything. New crate
  `image_edges` (std-only; better than 0.1 px on synthetic edges).
- **Fits**: through points (clicks snap to the nearest edge at subpixel,
  Alt places raw, a 4× loupe), fit to edge (click an edge: a dozen points
  round the click, extended both ways while the next stays within the
  chain's own noise band and the edge keeps its course; refuses a line
  on a curve or a circle covering under 8°), from an edge region (a box,
  median-consensus outliers, then least squares), and a spline through
  snapped points. Every fit reports σ and straightness or circularity.
- **Constructions**: midpoint, centre, intersection of two lines, circle
  through hole centres (bolt circle with each hole's stray).
- **Sheet alignment**: origin and +X by two snapped picks (the sheet rolls
  square), quarter turns, mirror (glass side to top view).
- **Object detection**: separate parts on the image by grey level, each
  with area, holes, centroid, equivalent diameter, largest and smallest
  caliper width, long axis and outline (`sketch.image_objects`).
- **Profile analysis**: an outline as radius against angle or distance
  along an axis; peaks counted by prominence with pitch and envelope
  (gear teeth, knurls).
- **Export**: the sketch at true scale as **DXF** (R2000, mm or inch
  declared, elements, edges thinned to 0.01 mm and notes on their own
  layers) and **SVG** (1 unit = 1 mm), and a CSV of elements and
  dimensions, each with a traceability line (scale, its source, frame).
  The Sketcher gains DXF and SVG export generally (the kernel writes
  DXF; `sketch.export_dxf`, `sketch.export_svg`).
- **Text notes** on the sheet, exported as text.

#### 6.4 Surface: fitting surfaces to scans

- **Fitted surface** (`SurfaceFeature` kind *FitScan*): a region (flooded
  across edges turning less than an angle, default 1°, or a marking),
  shape B-spline (degree 3, control net auto-sized from extent and noise
  or 3–20 per side, expanded by 35 % so neighbours intersect, fairing
  0.01), plane or cylinder; reports σ, worst point, net; live map.
  Kernel K24 (scattered fit on non-height-field regions, tolerance-driven
  refinement).
- **Auto constrain**: a fitted plane or cylinder within 0.5° of parallel
  or perpendicular to a reference is refitted with the direction held,
  within 1 mm of a parallel plane or axis snapped onto it, reported with
  the free-fit angle and σ before and after; "hold to" forces or forbids.
- **Trim by the scan**: a box around the scan cut by every fitted surface
  (extended), faces lying on the scan kept (area-sampled within a
  tolerance from the fits' σ) and sewn: closed becomes a solid with faces
  named after their surfaces, open stays a sheet with its open edges drawn
  red. Existing Trim and Sew do the cutting; the keep test is new.
- **Patch network** (*PatchNetwork*): click corners on the scan (sides
  follow the scan by a plane through consecutive corners), connect patches
  at shared corners and edges, drag corners and edges along the scan, mark
  creases, 3/5/6-sided regions split into quads from a centre, a boundary
  on a body edge with contact or tangency; suggested creases from sharp
  edges; all patches fitted in one solve with shared boundary poles,
  tangent across non-crease curves, spans per strip increased only where
  a patch misses by more than the tolerance; σ, worst tangent angle, worst
  gap per patch; a sheet or, closed, a solid; thicken. Kernel K25.

### 7. Host changes

#### 7.1 Import, bodies, tree

§1. The tree's body row shows scans with their badge; the Inspection
group lists elements, dimensions, tolerances, sections, alignments and
maps with edit, eye and delete keys and "Hide all / Show all".

#### 7.2 Scalar maps in the renderer

A `ScalarField` on `BodySubmission` (an `Arc<[f32]>` per vertex, a
range, a ramp id, band count, NaN as "unmeasured"): the vertex shader
reads the scalar, the fragment shader maps it through a ramp texture
with distinct end caps; ramp, range and bands are uniforms, so changing
them recolours without re-upload, and the scene fingerprint hashes them.
A second per-vertex vector attribute and a scale uniform drive the
**deformation animation** in the vertex shader. Picks write barycentric
coordinates for hover interpolation. The CPU picture renderer
(`thumbnail.rs`, `proof.rs`) draws the same maps, so agents see them.

The legend (ramp, histogram sharing its axis, statistics, tolerance
share) is one host widget any bench's map uses, driven by a
`MapLegend` the bench returns from a new `Workbench::scalar_maps`.

#### 7.3 Pins and labels

Pinned readings, dimension labels and note badges are screen-space
labels anchored on the part, de-overlapped by a bounded relaxation, faded
when behind the part (depth test against the scene readback).

#### 7.4 Section view with caps

The view's clipping plane caps watertight shells (edge parity) with a
flat cap in a per-body colour; open scans stay shells; nested parts
hatch. Nothing in the removed half picks. The Sketcher's per-sketch
section uses the same caps.

#### 7.5 Split view

Two viewports side by side with one shared camera (or independent for
picking correspondences), each with its own body visibility: scan on
the left, reference on the right. New in `render_wgpu` (two scene
passes into two rects) and `app/frame.rs`.

#### 7.6 Notes on geometry

Pinned notes on scan points, faces, edges and corners (anchored by
naming, as picks are), for the user to leave instructions and for an
agent to pin questions the user answers in place; resolved notes turn
grey with what was done. Notes from a file are flagged as content, not
instructions, when given to an agent. Saved with the document,
outside the undo history.

#### 7.7 Preferences

Units (mm/inch), STL units question, colour ramp, colour mode,
sharp-edge shading, map defaults (range rule, search distance, facing
limits), scanner profiles, marking brush default, line widths for
sections and 2D fits.

### 8. Printing: closing the loop

What turns measurement into better prints, beyond what general
inspection offers:

- **Compensation from a scan**: compare a print with its body; the bench
  fits per-axis scale factors (and an XY/Z split) by least squares over
  the matched points, separately estimates hole and boss compensation
  (radial offset of fitted cylinders against the model's), first-layer
  flare (the bottom band's outward offset), and warp (the bottom face's
  flatness and its corner lift). Results are a `PrintCompensation` saved
  in Printing preferences per printer and material, with the scan and
  date they came from.
- **Applying it**: export and Send to slicer apply the scale factors
  per axis and offset holes (an inverse offset of round faces through the
  kernel's Offset Faces, or for meshes a radial offset of fitted
  cylinders' regions); a toggle and a readout of what was applied.
- **Calibration coupons**: Start page and `print.coupon`: a parametric
  test part (holes and pins of graded sizes, a tolerance ladder, an
  overhang fan, a flat plate for warp) with the expected features already
  dimensioned; after printing and scanning, auto-align against the coupon
  body and every dimension fills in.
- **Fit check before printing**: a scanned mating part (a shaft, a
  device) against a designed housing: clearance map between the two
  (the Assembly's interference check on a scan).
- **Repair parts**: Mesh's trim-to-scan makes a glue-in piece for a
  broken part, exported for printing.
- **Printability on solids**: 3D wall thickness against the printer's
  minimum wall, overhang angle map, and both in the Inspect menu.

### 9. Commands, scripts and agents

Every capability is a command with notes, an example and a recipe,
recorded by the recorder, reached by Lua, the console, the palette and
MCP. Highlights:

- `inspect.fit` (kind, seeds as points, side, radius range, axis, box,
  known diameter, marking, cut-off), `inspect.construct`,
  `inspect.dimension`, `inspect.tolerance`, `inspect.align` (datum,
  auto, symmetry, best_fit, points, fine, reset), `inspect.section`
  (cut, shadow, through_axis, stack), `inspect.deviation`
  (reference or element, settings), `inspect.hotspots`,
  `inspect.volume_errors`, `inspect.thickness`, `inspect.report` (text,
  CSV, JSON with provenance), `inspect.map_export`,
  `inspect.export_step`, `inspect.compensation`.
- Scan reading: `scan.query` (vertices in a box or ball, by facing),
  `scan.nearest`, `scan.qa` (open rims numbered with circle fits, noise,
  thickness along an axis, bridged holes), `scan.segments` (planar
  regions largest first with σ, area, neighbours and joins; curved
  strips grouped into runs with a modelling suggestion; mirror twins),
  `scan.height_map`, `scan.outline`, `scan.rotational_pattern` (copies,
  pitch, agreement about an axis), `scan.draft` (patches against a pull
  direction), `scan.local_fit` (plane, twisted bilinear or quadric with
  principal curvatures, the simplest adequate).
- `mesh.*` (§3), `sketch.fit_chains`, `sketch.candidates`,
  `sketch.export_dxf/svg`, `sketch.image_*`, `design.snap_to_scan`,
  `design.read_thread`, `surface.fit_scan`, `surface.patch_network`,
  `doc.compare` (a body against the scan: share within, RMS,
  percentiles, worst spots, per-feature breakdown).
- **Pictures**: `view` and `doc.picture` draw maps, pins, sections and
  the section view; a **section picture** (the cut as points on a mm
  grid with the sketch or model cut over it, far points in red, several
  parallel cuts side by side) is the agent's main proof while modelling.
- **Guides and skill**: `docs/INSPECT.md`, `docs/MESH.md`,
  `docs/REVERSE.md` (and recipes for each workflow), and an agent skill
  `printcad-reverse-engineering` (the loop: check the scan, classify the
  part and set the tolerance, never finer than three times the noise,
  ask the user once, build as a designer would with sections, relations
  and round numbers, loop on the deviation map and volume errors, report
  each feature as measured, partly supported or inferred, and say "not
  done" when the target is not met). Part types with default targets
  (machined ±0.02 mm, printed ±0.1–0.2 mm, cast ±0.5 mm; within-tolerance
  targets 90 % analytic, 85 % moulded, 75 % freeform).
- **Robustness**: every new command joins the hostile-arguments sweep
  and the every-example test.

### 10. Files and persistence

- Scans, markings and mesh-feature results are container entries
  (`mesh/<uuid>.bin`, `marking/<uuid>.bin`), never JSON; the scan's
  source file is an asset (re-readable, exportable). Mesh history results
  are derived and rebuilt on load, not saved (as solids are), except the
  start mesh.
- New fields default (`#[serde(default)]`), new feature kinds belong to
  new benches; nothing changes what old data means, so
  `CONTAINER_FORMAT` does not move.
- Derived results (maps, fits, edge chains) are recomputed on open, so a
  better algorithm improves old documents; fits are deterministic and
  the corpus holds them (§11).
- Large documents: a 5 M-triangle scan is ~120 MB indexed; the container
  stores it zstd-compressed when the name says so, and recovery copies
  skip unchanged entries.

### 11. Testing and accuracy

- **Synthetic scans**: a test crate (`scan_fixtures`) that samples exact
  shapes (box, ball bar, stepped bore, drafted boss, bracket) into meshes
  with a seeded noise model (Gaussian normal noise, rounded edges, holes,
  islands, spatter) and known truth.
- **Closed forms**: every fit recovers its parameters within a bound set
  by the noise and sample count; ICP returns a randomly displaced scan to
  its pose within 1 µm; the symmetry plane of a symmetric part within
  noise; deviation of an offset copy equals the offset; thickness of a
  plate equals its thickness; smoothing keeps a noisy ball's radius within
  1 µm; offsetting a box moves faces by the distance and corners by √3
  times; hollowing a 100 mm ball with a 2 mm wall gives its cavity
  volume within 0.2 %; reduction holds its tolerance two ways at 99.9 %;
  subpixel edges recover synthetic lines within 0.1 px.
- **Reference parts**: a real scan of a ball bar and of a printed coupon
  in the test data (small, decimated) with measures from a calibrated
  instrument held as in the release corpus.
- **Corpus**: each release saves an inspection document's report; later
  builds must reproduce it.
- **Performance**: budgets (§13) checked by ignored benches run on
  request, like `import_bench`.
- **UI**: before/after pictures in each PR (the project rule); a scripted
  check per tool through `PRINTCAD_WIDGET_MAP`.

### 12. Kernel requests (ogeom issues to file)

Each is an issue on the kernel repository with the API, semantics, a
minimal repro and an acceptance test, per the kernel gap protocol;
printCAD wires the call and marks its test `#[ignore = "kernel: …
(ogeom-rs#N)"]` until it lands.

| # | Request | Proposed API (sketch) | Acceptance |
| --- | --- | --- | --- |
| K1 | Best fits that always answer, with residuals | `algo::fit::{plane, sphere, cylinder, cone, torus, circle3, line3, circle2, line2}(points, normals: Option<&[Vector]>, &FitOptions{ robust_start, cutoff: Cutoff::{All, Sigma(f64)}, fixed: FitFixed{ radius, axis, direction }, guess }) -> Fit { shape, sigma, form: (min, max), used: BitVec, residuals: Vec<f64> }` | Synthetic shapes with Gaussian noise recover parameters within 3σ/√n; a fixed radius holds; outliers past the cut-off are excluded |
| K2 | Minimum-zone, maximum-inscribed, minimum-circumscribed fits | `algo::fit::zone::{flatness, circularity, cylindricity, sphericity, straightness}`, `inscribed_circle/cylinder`, `circumscribed_*` | Known zone widths on constructed point sets to 1e-9 |
| K3 | Signed distance from points to a mesh or B-rep, batched, facing-aware | `algo::distance::{to_mesh, to_shape}(points, normals, &DistanceOptions{ max, facing_limit, signed: Pseudonormal }) -> Vec<Option<(f64, Point, FeatureKind)>>` | Offset copies read the offset; no sign flips at seams; 1 M points/s on a 1 M-triangle mesh |
| K4 | A public spatial index over meshes and shapes | `mesh::Bvh::build(&Triangulation)` with `nearest`, `within_ball`, `ray`; kept per mesh | Agrees with brute force; build 1 M triangles < 1 s |
| K5 | Rigid registration (ICP) | `algo::register::{icp(source, target, &IcpOptions{ point_to_plane, cutoff: Median(k), max_distance, facing_limit, weights }), absolute_orientation(pairs)}` → `Registration { transform, rms, pairs, converged, degenerate_directions }` | Random displacement returns within 1 µm; a one-sided selection reports its free directions |
| K6 | Normal-histogram face directions and principal frames | `algo::orient::{face_directions(mesh, smoothing), principal_frame(mesh)}` | A drafted box finds its six directions within 0.05° |
| K7 | Plane section of a mesh into polylines with material side | `mesh::section(&Triangulation, &Plane) -> Vec<Chain{ points, out_of_material: Vec<Vector>, closed }>` | Closed scans give closed chains; side correct on thin walls |
| K8 | Mesh silhouette onto a plane, outer boundary, fitted to lines/arcs/circles outside a band | `mesh::shadow(&Triangulation, dir) -> Vec<Loop>`, then `algo::fit_outline(loops, play, tolerance, round_below) -> Vec<Wire2d>` | Every piece within [play, play + 2·tol] of the exact silhouette |
| K9 | Curvature-only tessellation mode | `Deflection { length: None, angle, chord }` honoured with no length cap | A cube meshes to 12 triangles |
| K10 | Mesh topology editing on an indexed mesh | `mesh::Editable` (half-edge) with `components`, `remove(faces)`, `flip(faces)`, `rims()` | Round trips; manifoldness kept |
| K11 | Plane cut of a mesh with flat caps | `Editable::cut(&Plane, keep, cap: Cap{ bridge: f64 }) -> CutReport` | Nested loops capped; gaps bridged as configured |
| K12 | Hole filling | `Editable::fill(rim, Fill{ fairing: Curved | Flat, refine })` | Min-max dihedral DP up to 500 edges; scan vertices fixed |
| K13 | Feature-preserving smoothing and mesh curvature | `Editable::smooth(&Smooth{ size, max_move, keep_edges })`, `mesh::curvature(mesh, size)` | A noisy ball keeps its radius within 1 µm |
| K14 | Mesh offset with fold repair | `Editable::offset(distance, region)` | A box offset by d: faces d, edges √2d, corners √3d |
| K15 | Exact mesh booleans and mesh–solid booleans | `mesh::boolean(a, b, op)`; `mesh::boolean_with_shape(mesh, model, shape, op, chord)` | Closed results; exact on closed inputs; 1 M triangles in seconds |
| K16 | Hollow and thicken meshes | `mesh::hollow(mesh, wall, &GridOptions)`, `mesh::thicken(mesh, d, side)` | Cavity volume within 0.2 % on a ball |
| K17 | Decimation verified both ways with pinning | extend `mesh::simplify` with `Target::Verified{ tolerance, share: 0.999 }` | Two-way 99.9 % within tolerance |
| K18 | Cut a solid by a thickened mesh region | `algo::cut_by_mesh_region(model, shape, mesh_region, offset)` | A fractured block's piece within 0.5 % volume |
| K19 | Volume comparison of a mesh and a solid | `algo::volume_difference(mesh, model, shape, cell) -> Vec<Region{ volume, depth, kind, box }>` | A missing boss is found with its volume |
| K20 | Batched ray casting and inscribed-sphere thickness | `mesh::Bvh::rays(origins, dirs, max) -> Vec<Option<Hit>>`; `algo::thickness(mesh_or_shape, &Thickness{ method: Ray{cone, count} | Sphere, facing_limit, max })` | A plate reads its thickness; 150 k vertices/s |
| K21 | STEP wireframe and construction geometry write | `write_step` accepts points, lines, axis placements and trimmed curves in a geometric set | Round trip through the reader |
| K22 | PMI write completeness | Datum reference frames, modifiers, profile and position tolerances | Round trip of each kind |
| K23 | Fit a chain of lines and arcs to a 2D polyline | `algo::fit_segments_2d(points, tolerance, &SegmentOptions{ corners, axis_snap, min_bow }) -> Vec<Segment2>` | A noisy slot outline gives four pieces with exact tangency |
| K24 | Scattered surface fitting beyond height fields | `geom::fit::fit_surface_scattered` with parameterisation by projection onto a seed surface (cylinder, sphere), tolerance-driven control refinement | A half cylinder fits within noise |
| K25 | Patch network fitting with shared poles and tangency | `algo::fit_network(patches, curves, scan, &NetworkOptions)` | Tangent within 0.001°, gaps ≤ 1 µm |
| K26 | Binary PLY (vertex colours, normals, scalars), XYZ/PTS point clouds, OBJ with MTL/texture colours, 3MF colours and textures, slicer-project volume roles | `io::mesh_formats` | Round trips; colours sampled per vertex |
| K27 | Public statistics on residual sets | none needed: `metrology` does it app-side | — |

K27 is listed to say where the line is: statistics, GD&T evaluation,
report building, design-value rounding and colour ramps are not geometry
and stay in printCAD's `metrology` crate. Mesh topology editing (K10–K16)
could alternatively live in a printCAD `mesh_kit` crate; this RFC asks
the kernel first because the solid side (K15, K18) needs the same mesh
structures, and the protocol prefers one home.

### 13. Performance budgets

| Operation | Size | Budget |
| --- | --- | --- |
| Import a binary scan | 2 M triangles | < 3 s to first frame |
| Orbit | 5 M triangles | ≥ 30 fps (LOD while moving) |
| Click-to-fit preview | any | < 100 ms |
| Deviation map | 2 M scan vertices, 100 k reference triangles | < 2 s |
| Best fit | 1.5 M triangles | < 5 s, streamed |
| Auto-align | 2 M triangles | < 1.5 s |
| Wall thickness, ray | 1 M vertices | < 7 s |
| Volume errors | 1.5 M triangles | < 15 s |
| Reduce | 1.4 M → ±0.1 mm | < 5 s |
| Hollow | 1.3 M triangles | < 5 s |

All heavy work runs on the kernel worker or a job thread with progress
and Cancel, never in a frame.

## Alternatives considered

- **One "Scan" bench holding everything** (measure, mesh, reverse). It
  would duplicate the Sketcher, Design and Surface. Reverse engineering
  is modelling; it belongs with the modelling tools, which gain scan
  awareness instead.
- **An edited copy beside the scan** (rather than mesh history). Simpler,
  but a second kind of state outside the feature model, with its own
  undo and save rules. The history reuses every mechanism printCAD has.
- **A workbench package** (wasm). The scale (millions of vertices
  crossing the boundary every rebuild), the renderer and marking changes
  and the kernel calls argue for native benches; the host services added
  here (marking, scalar maps, notes) are written so packages can use them
  later.
- **App-side mesh algorithms**. Faster to start, but the kernel gap
  protocol is clear, and the solid-side operations need the same mesh
  structures.

## Risks and open questions

- **Kernel scope** (K1–K26) is large; the milestones are ordered so each
  ships with what the kernel has, the rest marked ignored.
- **Memory** on 32-bit targets with very large scans: the release
  targets are 64-bit; the container already keeps meshes apart.
- **Determinism** of fits across platforms (floating point, threads):
  fixed seeds, ordered parallel maps (`map_ordered`), and the corpus.
- **Naming of scan regions** when a mesh edit renumbers vertices: mesh
  features keep vertex ids stable where they can (the K10 editable mesh
  keeps a correspondence), and markings map through it.
- **Inch display** touches every field in the app; it is staged as its
  own milestone.
- **GD&T second version** needs datum reference frames that carry into
  Design's PMI export; its design is left for a follow-up RFC.

## Milestones

1. **Scan bodies**: import (formats, units, colours, point clouds),
   export, large-mesh LOD, marking service, scalar-map renderer pass and
   legend. Kernel: K4, K26.
2. **Measure**: Inspection bench with fits, constructions, dimensions,
   first-version tolerances, reports. Kernel: K1, K6.
3. **Align and compare**: datums, auto-align, symmetry, best fit, picked
   points, fine fit, split view; deviation map, hot spots, element maps;
   wall thickness on scans and solids. Kernel: K3, K5, K9, K20.
4. **Sections and sheets**: sections, shadows, stacks, section view caps;
   Sketcher scan sections, fit chains, soft data, design values;
   image edges, calibration, DXF/SVG export. Kernel: K7, K8, K23, K21.
5. **Mesh workbench**: islands, delete, flip, cut, fill, smooth, offset,
   reduce, then booleans, hollow, trim-to-scan. Kernel: K10–K18.
6. **Model from the scan**: live deviation in Design, snap to scan,
   extract, read rounds/holes/threads, fitted surfaces, trim by scan,
   patch networks, notes on geometry, the agent skill. Kernel: K24, K25,
   K19.
7. **Printing loop**: compensation, coupons, fit checks, printability.
8. **Second-version GD&T**, inch display, report documents. Kernel: K2,
   K22.

Each milestone ends with its guide, recipes, command examples, synthetic
tests at closed forms, and pictures in its pull requests.

## Acceptance

- A scan of a printed calibration coupon, imported, auto-aligned against
  the coupon body and compared, reports every coupon dimension with
  PASS/FAIL and yields per-axis scale and hole compensation that, applied
  to the next print, brings its deviation RMS down (measured on a second
  scan).
- A ball bar scan's sphere centre distance matches a calibrated value
  within the scan's noise.
- A broken bracket's scan is reverse engineered by an agent through the
  commands into a Design body within the target share of its tolerance,
  with the report naming each feature's evidence.
- A flatbed scan of a gasket becomes a calibrated sketch exported as DXF
  whose dimensions match calipers within 0.05 mm.
- Every new command runs in the every-example and hostile-arguments
  tests; every kernel request is filed with its acceptance test.
