# Release notes

Each release is a `## <version>` heading, its topics `### <topic>`, and one
bullet per change. The start page's What's new shows these, the running
version first.

## 0.1.0

### Documents
- Several documents open at once, one per tab, each with its own undo history and camera.
- A document is served by its own local document server; saving never holds the window.
- Export the solids to STEP, STL or 3MF from the File menu; the mesh formats use the tessellation tolerance you choose.
- Send to slicer (Ctrl+P) opens every visible body in your slicer, set in Preferences › 3D printing.
- Recent documents show a rendered preview, saved with the document.

### Sketcher
- Lines, polylines with tangent arcs, rectangles, polygons, circles, arcs, ellipses (by centre or three points), arcs of ellipses, B-splines and slots.
- Geometric and dimensional constraints, solved live, with a message for every conflicting or redundant one.
- Trim, extend, split, fillet, offset, mirror, move, rotate, scale and arrays.
- Carbon copy brings another sketch's geometry in; merge makes one sketch of several.
- External geometry projects a solid's edges into the sketch as fixed references to constrain against, kept up to date when the solid changes.
- Rendering order puts construction or normal geometry on top.
- Drawing snaps to the origin and the two axes as it does to drawn geometry, and pins the new point there.

### Part Design
- Pad, pocket, revolution, groove, loft, pipe, helix and primitives, additive and subtractive, and booleans between bodies.
- Holes to standard sizes, fillets and chamfers on picked edges, draft, thickness, and linear, polar and mirrored patterns.
- Datum points, lines and planes, and local coordinate systems whose planes carry sketches.
- A body's volume, surface area and centre of mass, exact wherever its faces have a closed form.

### Assembly
- Bodies can be moved and turned, and keep their own geometry as it was made.
- Joints place one body against another: mate two flat faces, with a gap or facing the same way, line up a pin with a hole on one axis, or hold two faces at an angle.
- Joints solve when made or edited and when a body moves, and undo as one step with the moves they cause.
- Export writes each body where it sits.

### Import
- STEP and IGES import as bodies, assemblies placed as their files say.
- STL, OBJ and 3MF import as meshes, and a mesh converts to a solid on request.
- Every imported body is checked; a broken one shows red and the kernel repairs it on request.

### View
- Faces and edges select one by one; a double click takes the whole body.
- Shaded, shaded with edges, and wireframe; orthographic or perspective with a field of view to taste.
- A clipping plane cuts the view across X, Y or Z.
- A 6-DoF mouse steers the view.

### Keyboard
- Every shortcut can be changed in Preferences › Keyboard, which also warns when two commands share a key.
- Sketch tools have keys of their own: L, P, A, C, R and T, and M switches a polyline between lines and arcs.
- Menus, tooltips and the command palette show the keys in effect.
