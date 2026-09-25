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
- Undo inside a sketch takes back the last thing done, not the whole editing session.
- Sketch edits keep the shape: moved, turned, scaled or mirrored geometry stays where it was put, copies keep their constraints, fillets stay tangent, trimmed and split lines keep their line, dragged points stay under the cursor.
- Polygons, slots and centred rectangles are held in shape as they are drawn: a polygon regular on its circle, a slot's sides tangent to its caps, a centred rectangle symmetric about its centre.
- Clicks that do nothing say why, overlapping constraint icons spread out to be clickable, and every preview matches what the click makes.
- Snapping works the same in every drawing tool and every click: to endpoints, centres, the origin, crossings, the middles of lines, square to a line or touching a circle from the last point, curves and axes, and level or plumb with the last point. Each has its own marker and name at the cursor, the click lands exactly where the marker is, and the point stays there by a matching constraint.

### Part Design
- Pad, pocket, revolution, groove, loft, pipe, helix and primitives, additive and subtractive, and booleans between bodies.
- Holes to standard sizes, fillets and chamfers on picked edges, draft, thickness, and linear, polar and mirrored patterns.
- Datum points, lines and planes, and local coordinate systems whose planes carry sketches.
- A body's volume, surface area and centre of mass, exact wherever its faces have a closed form.
- While a feature is edited, what it adds or cuts shows see-through in its own colour over the body without it, set in Preferences › Display.

### Assembly
- Bodies can be moved and turned, and keep their own geometry as it was made.
- Joints place one body against another: mate two flat faces, with a gap or facing the same way, line up a pin with a hole on one axis, or hold two faces at an angle.
- More joints: a hinge that leaves only the turn about an axis, a slider that leaves only the slide along one, a fixed joint that carries a body with another, parallel, perpendicular and distance between flat faces, and a round face resting on a flat one.
- Joints that ask for an axis take a round face or an edge: a hole's rim gives the hole's axis.
- Drag a jointed body with the mouse: it follows as far as its joints let it, a door swinging on its hinge, stops where it would run into another body, and undoes as one step.
- Record saves a driven joint's sweep as an animated PNG, and the parts list saves as a CSV file.
- Check interference (I) finds every pair of visible solids that share material, with how much, and draws what they share over the scene; it runs beside the window, with progress and Stop.
- Exploded view (E) spreads the bodies out from the middle of the assembly, and the parts list (B) counts every part with its size, ready to copy for a spreadsheet.
- A hinge's angle and a slider's position can be driven, by a number or a formula, or kept within limits; Play sweeps a driven joint through its range to show the motion.
- Grounding keeps a body where it is; the whole assembly solves together, rings of joints included, and the status bar says what each body may still do.
- Joints solve when made or edited and when a body moves, and undo as one step with the moves they cause.
- Export writes each body where it sits.

### Import
- STEP and IGES import as bodies, assemblies placed as their files say.
- STL, OBJ, 3MF, PLY, glTF and VRML import as meshes, and a mesh converts to a solid on request.
- Every imported body is checked; a broken one shows red and the kernel repairs it on request.
- The dimensions, tolerances, datums and notes a STEP or IGES file carries show over the model and in the tree, and each body lists the layers it is on; View › Annotations turns them off.

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
- Variable sets hold named values defined by formulas (`wall = 3 * Printer.nozzle`), made from the tree's document row and edited in the property panel when selected.
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
- A document's chats are kept with its file: open it again and they continue where they left off, the conversation replayed.
- `printcad --mcp` serves the running application to any MCP client. See docs/AI.md.

### Workbench packages
- Workbenches others made install from a `.pcbench` file in Preferences › Workbench packages: tools, features with parametric solids, task panels whose numbers take formulas, viewport drawing, commands for scripts and agents, and a settings page.
- Each package runs sandboxed: it reaches its own folder and nothing else unless you allow it to save files, run the programs it ships or use the network, and one that hangs or crashes is restarted, then turned off, without harming the app.
- Packages install from a GitHub repository's releases as well as from a file, and the app checks for and installs their updates.
- Installing, updating, removing or turning a package on or off takes effect at once, without restarting: its workbench joins or leaves the list, and its features in open documents rebuild with the version now running.
- A document opened without the package it used keeps that package's features and shapes, and says which package they need.
- The SDK and a gear workbench to start from are in `sdk/`; see docs/PLUGINS.md.

### Keyboard
- Every shortcut can be changed in Preferences › Keyboard, which also warns when two commands share a key.
- Every workbench has default keys: a letter picks a tool, Shift and a letter its partner (a sketch constraint, or the subtractive form of a Part Design feature).
- 0 to 6 give the standard views, O and P the projection, Shift+F fits the selection, Ctrl+R recomputes, and Space shows or hides the selected tree row.
- Menus, context menus, toolbar tooltips and the command palette show the keys in effect.
