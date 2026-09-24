# Project status

What is built, by area. The plan for what comes next is in
[docs/plan.md](docs/plan.md).

## Application

- Wayland or X11 window, Vulkan renderer with MSAA, GPU picking and a cached
  scene, frames drawn only when something changes.
- Tabs, one document each, and a start page with recent documents,
  previews, examples and release notes.
- Command palette, Preferences with a page per workbench, log panel.
- Design system (`ui_kit`) used by every panel.

## View

- Camera with orbit, pan, roll, zoom to cursor, pivot picking and fitted
  near and far planes. See [camera_system.md](camera_system.md).
- Axis presets, orientation cube, standard views, draw styles, field of view,
  clipping plane, measure tool, print bed outline.
- 6-DoF mouse support through the `sixdof` crate.
- Selection of faces, edges and whole bodies, with hover highlighting.

## Documents

- `.prtcad` files: a tar archive with the features, kernel shapes, source
  files and a preview.
- One operation per edit, undo by inverse operations, and a document server
  process per document.
- STEP and IGES import, STL, OBJ and 3MF import as meshes, and conversion of
  a mesh to a solid.
- Shape health checks and repair for imported bodies.
- Export to STEP, STL and 3MF, and Send to slicer: every visible body
  written to a temporary 3MF or STL and opened with the slicer command from
  Preferences › 3D printing.

## Sketcher

- Points, lines, polylines, rectangles, polygons, circles, arcs, ellipses,
  arcs of ellipses, B-splines and slots.
- Constraints solved live, with degrees of freedom and conflict reports.
- Trim, extend, split, fillet, chamfer, offset, mirror, move, rotate, scale,
  arrays, carbon copy and merge.
- External geometry: a solid's edges projected in as fixed references.

## Part Design

- Pad, pocket, revolution, groove, loft, pipe, helix and primitives, in
  additive and subtractive forms.
- Hole, fillet, chamfer, draft, thickness, patterns, mirror and booleans.
- Datum points, lines, planes and local coordinate systems.
- Volume, surface area and centre of mass of a body.

## Assembly

- Bodies have a placement; their own geometry stays in their own frame.
- Joints between bodies: mate two flat faces (with a gap, or facing the same
  way), align two round faces on one axis, and hold two faces at an angle,
  solved into placements.
- Move a body by numbers.

## Not built yet

- More joint kinds: gears, limits.
