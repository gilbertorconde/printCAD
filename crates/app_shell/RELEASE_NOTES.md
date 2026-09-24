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

### Scripting
- Lua scripts reach every command of the application and the workbenches under `pc`: sketches and constraints, every Part Design feature and datum, assembly joints, rebuilding, measuring, faces, files and export.
- The console completes command names with Tab, takes several lines, and keeps its history; Save as script turns a session into a script.
- Every `.lua` file in the scripts folder shows in the Scripts menu, the toolbar and the palette, and takes a key.
- `printcad --script build.lua` runs a script without a window, for batch work.
- Scripts run on a thread of their own: the window stays live and the status bar shows the running script with a Stop button.
- Scripts › Record… writes what you do as a script that does it again: every tool ends in the command it stands for, so the recording is the list of those commands, with what it makes named for later lines.
- A script run is one undo step. See docs/SCRIPTING.md.

### Variables and formulas
- Variable sets hold named values defined by formulas (`wall = 3 * Printer.nozzle`), edited in the Variables panel (Windows › Variables).
- Any number can be a formula: a pad's length, a hole's diameter, a sketch dimension, a joint's gap, a datum's offset, in the property panel, the task panels or the sketch's dimension editor.
- Units are checked (a length field refuses an angle) and typed values take units (`1 in`); names complete as you type.
- Formulas read other objects' numbers (`Pad.length`, a named sketch dimension); renaming anything rewrites the formulas that use it.
- Configurations give chosen variables other values (Small, Large); switching rebuilds, and export can write every configuration.
- Scripts and agents use them through `var.*`, `config.*` and `doc.set_formula`. See docs/VARIABLES.md.

### AI agents
- The Assistant panel (Windows › Assistant) chats with AI agents that speak the Agent Client Protocol, set up in Preferences › AI agents; several chats run at once, each in its own tab.
- Agents work the document through the same commands scripts use, and see the view and the log.
- Every change an agent asks for waits for your OK unless you allow the chat; each is one undo step.
- Attach files or a picture of the view to a message from the "+" in the chat's bar, by pasting copied files, or by dropping them on the panel.
- The bar under a chat's box sets what the agent offers, such as its permission mode, model and effort, and new chats with that agent keep the choice.
- `printcad --mcp` serves the running application to any MCP client. See docs/AI.md.

### Keyboard
- Every shortcut can be changed in Preferences › Keyboard, which also warns when two commands share a key.
- Every workbench has default keys: a letter picks a tool, Shift and a letter its partner (a sketch constraint, or the subtractive form of a Part Design feature).
- 0 to 6 give the standard views, O and P the projection, Shift+F fits the selection, Ctrl+R recomputes, and Space shows or hides the selected tree row.
- Menus, context menus, toolbar tooltips and the command palette show the keys in effect.
