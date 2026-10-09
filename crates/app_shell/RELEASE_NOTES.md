# Release notes

Each release is a `## <version>` heading, its topics `### <topic>`, and one
bullet per change. The start page's What's new shows these, the running
version first.

## 0.8.1

### In your browser
- Large assemblies import, save and reopen in the browser: the 165 MB, 1,189-body Doom 350 assembly imports in about 3 minutes and saves to 184 MB. Its import used to stop at "Preparing… bodies" for good, and saving it crashed the page.
- When the kernel stops in the browser (out of memory, say), the task ends with a message saying why rather than waiting forever.
- The browser keeps saved documents compressed, so they take far less of its memory and storage.

### Documents
- A document keeps each body's mesh as a compact entry of its own, so large documents save smaller and faster. A document saved by this version opens in 0.8.0 with those bodies' meshes missing until they are imported again; earlier documents open as before.
- A compressed `.prtcad`, as the browser downloads one, opens on the desktop with its preview.

### Kernel
- Built on ogeom 0.9.15: parts far from the origin keep their faces and features through conversion, sewing, chamfers, pipes, meshing, booleans and healing, mesh conversion depends far less on the order of a mesh's triangles, and mass properties are exact under a scaling placement.

## 0.8.0

### In your browser
- printCAD runs in a web browser, with nothing to install: the website's Try in browser opens it. It draws with WebGPU where the browser has it, WebGL2 otherwise.
- The kernel works in the page's background workers, so the page stays responsive while bodies build, several at once; where the browser allows it, each worker uses several threads.
- The browser keeps your documents: Save keeps one under a name, the start page lists them with their previews, and a card's menu deletes one. File › Download a copy saves a document to disk, and Open reads any `.prtcad` file.
- Autosaved copies come back on the start page after the page was closed, and leaving the page with unsaved edits asks first.
- STEP, IGES and mesh files import, and exports, pictures and recorded scripts download.
- The script console runs Lua, workbench packages install from a file or from the workbench store, and a 6-DoF mouse connects from Preferences › Input › 6-DoF mouse (in Chrome and Edge).
- Left to the desktop app: Send to slicer, the AI assistant, update checks, linking parts from another printCAD file, the scripts folder, and package installs from a GitHub address. A package in the browser has no folder of its own, no network and no helpers.

### Display
- A new renderer: Vulkan on Linux, Metal on macOS, DirectX 12 or Vulkan on Windows. The scene, edges, picking and section cuts look and behave as before.
- Edges draw at the width you set on every graphics API.
- Preferences › General › About names the graphics API in use, and `WGPU_BACKEND=gl` runs printCAD on OpenGL where Vulkan is not available.

### Navigation
- Orbiting and panning follow the drag from where it started, so the same movement of the mouse always gives the same view, however fast the pointer reports.
- Orbit turns about the world's up and stops at the poles rather than rolling over; a saved Camera up setting keeps free rotation.
- Fit uses the narrower side of the view, so a tall window frames the model too, and frames models larger than the wheel zooms out to. Switching between perspective and orthographic keeps the scale at the pivot.

### Assembly
- Move body has handles on the body: arrows, plane squares and a view disc slide it, rings turn it. The panel's Handles choice or Shift+G switches between moving and turning, a held handle shows its value, Shift steps it, and Escape puts the body back.

### Closing
- Closing a tab, the window or the app with unsaved edits asks in printCAD's own window (Save, Don't save, Cancel), quitting asking for each tab in turn. On Linux it no longer needs zenity: without it the close button did nothing.

### Kernel
- Built on ogeom 0.9.14, which runs in a browser. Sections through shallow crossings state their error truthfully, and inside-or-outside checks answer where a ray only grazes a face.

## 0.7.1

### Kernel
- Built on ogeom 0.9.13. The interference check works again on a worm passing through a bearing.
- Thickness and shells work on solids with rounded edges and corner balls, and a drill or cut that the kernel refused on a closed shell now builds.
- Faster across the board in the kernel's measurements: holes and cuts into large drilled parts, volume, mass and filament figures (about twice as fast), shape checks and repairs, STEP and IGES import, pipes along circular paths, thickened spline surfaces and primitives.
- A solid saved by 0.7.0 whose seams were stored loosely is reported by the shape check and mended by Repair shape.
- A small edit to a large part rebuilds in about half the time: the check for an edit that made the same solid no longer meshes every face, and the states a body's builds keep hold only what the solid still reaches.

## 0.7.0

### Kernel
- Built on ogeom 0.9.9: a small cut into a large part costs what it touches (a hole added to a plate of 576 holes takes 0.3 s in place of 1.1 s), and features on imported parts whose fillets carry small rings build, half domes sew into an open dome, mirrored sheets shade the right way, lofts and domes thicken, fillets take a fraction of the time, and a body's saved shape is a fraction of the size (1.6 MB in place of 39 MB for a plate of 576 holes).
- Known: the interference check fails on a worm passing through a bearing, its worm built by this kernel; a fix is on its way in the kernel.

### Surfaces
- A curvature map (Gaussian, mean, largest or smallest) and zebra stripes paint a body with how its surfaces bend, and Check continuity tells curvature continuous joins (G2) from tangent ones.
- Fillet between surfaces rounds between two faces that share no edge, lofts follow guide curves, and a sweep can ride two rails.
- Edges picked on other bodies, solids too, are curves a surface builds from, following those bodies as they change and move.
- A face picked gives its edges at once to the surface tools that take curves or edges.
- Check continuity measures a solid's far ends where they stand: a padded block's top edges no longer show as gaps the pad's length wide.
- An extruded surface whose curve runs along its direction (a line extruded along itself) fails with a message saying so, rather than leaving the body empty.
- A swept surface runs from the end of its path the profile sits by, rather than on past the path's far end when the path was drawn toward the profile.

### Sketcher
- An ellipse showing its foci drags through a circle: a minor radius pulled past the major makes it the major, the foci sliding through the centre onto the new axis.

### Design
- The gear, sprocket and shaft tools ask for their plane as a new sketch does, the clicked face offered first; on a face of the body's own solid the sketch follows it, centred where it was clicked, and the view turns square to it. sketch.new takes a generator.
- The refine after a feature merges only what that feature made with its neighbours: rebuilds take a third to a half less time, and a split kept on purpose stays.
- Borrowed faces are stop faces of an up-to-shape pad or pocket and targets a revolution or groove turns until, and an existing sketch maps onto one from its tree menu (design.map_sketch), following it.
- A chamfer by two distances, or by a distance and an angle, keeps each distance on its own side all along a tangent chain, where the faces change from a flat side to a round and on.
- A gear's root is the one a rack cutter leaves, undercutting the flanks of a pinion of few teeth.
- Gears and sprockets take a keyway in the bore, and the Keyway generator draws a shaft's key slot to pocket, both sized by the standard key (DIN 6885) unless given.

### Printing
- The parts list says how many of each part to print (one per body unless you set another count), the volume of a piece and the filament it takes in the material you print in (PLA, PETG, ABS, ASA, TPU, Nylon, PC or a density of your own), and adds up the whole print. asm.print_material sets the material, asm.part a count.
- A hole takes a nut trap: a hexagonal pocket for a captive nut at its mouth or where it ends, sized from the ISO 4032 or DIN 934 nut of its metric thread with a clearance, or of a size of your own, turned as you like. design.hole takes it as nut_trap.
- File › Print layout lays every part flat on the bed, on its largest face it can stand on, as many copies as the parts list prints, packed in rows with a gap, extra plates beside the bed; the view shows the layout while the task is open and the model stays where it is. Export and Send to slicer write the layout when asked (Laid out for printing, Preferences › Printing › Send the print layout). doc.print_layout answers it to a script, file.export and file.send_to_slicer take layout = true.

### AI assistant
- doc.faces and doc.edges give a moved body's faces and edges in its own frame too (frame = "body"), as features take them.

### View
- A new view, and Fit all, look at the model from above at a corner; they looked up at it from below. An opened document is framed once its bodies have their shapes, and its saved preview is drawn from above too.
- A mouse release a panel, a dialog or a bench took no longer leaves a drag held, which turned the next right-button pan into a roll.

## 0.6.0

### AI assistant
- Agents find commands by the words of a task: a search that knows CAD synonyms ("hole", "round the edges", "shell a box"), and a describe that copes with bare names and typos.
- Every command an agent can run carries notes on what it refuses or does silently, related commands and a working example; the test suite runs every example.
- Seven recipes for common parts: a plate with a hole, a bracket, a bushing, a flange, a configured plate, a hinged arm and surface shells.
- The view tool draws fixed views framed to the bodies asked about, with outlines, highlighted faces or edges, markers, a section and x-ray; doc.picture writes the same picture to a file.
- A Lua script answers what it returns, and doc.edges lists a body's edges as fillets and chamfers take them.
- A script that stops with an error takes back what it changed.
- A chat comes back with its document, even one started before the file was saved or after it moved.

### Surfaces
- Sweeps along a line into a tangent arc, fills of loops that leave their plane, a tangent cap on a tube, thickening across creases and between arcs, and exact measures of sewn boxes now build.
- Extend works on extruded and revolved surfaces, and Split lands a sketch on a curved face as seen square to it.
- A step that fails leaves the body its history before it, and the steps after it say they were not built.
- Deleting a step rebuilds its body.
- surface.set changes a step, surface.check reports how faces meet, and every step's command takes its fields by name.
- Open surfaces report no volume.

### Design
- Delete faces on a pocketed bore, moving a bore sideways and the centre line of a pipe with sharp corners work.
- A refined bracket and part-turn helices measure exactly.
- A formula that fails stops its feature's build with the formula's error.
- Clearing a formula keeps the number it gave.

### Sketcher
- Geometry taken from another sketch follows it.
- A sketch whose constraints conflict fails the features built on it instead of padding a distorted shape.

### Assembly
- Its panels use the same declared widgets packages use.
- Motion over time and Save assembly state have their tools and keys.
- A grounded body's joint moves the body at its other end, and joints that cannot hold are named.
- An interference check goes on past a pair it cannot check, and lists it.
- The mass panel uses each body's material.

### Workbench packages
- A CAM example package: a pocket toolpath worked out as a job, drawn over the model and saved as G-code.
- New panel widgets: header, value, row, hint, slider, an editable table and a sheet.
- A package can read a sketch's closed loops, and can declare the parts it makes as bought, which the parts list and exports respect.

### Documents
- Pictures and previews show face colours.
- A frozen body keeps its solid through a recompute, and a refused reorder changes nothing.

## 0.5.0

### Surfaces
- A Surface workbench: extruded, revolved, planar, filled, ruled, lofted, swept, offset and blend surfaces, from sketch curves (open or closed) and from picked edges.
- Sew joins a body's surfaces, across a gap you set if they do not quite meet, and a shell that closes becomes a solid.
- Surface fillet, Extend, Split, Trim by plane, Thicken and Mirror work on the surfaces you have.
- A filling takes any number of sides and meets the surfaces around it touching, tangent or curvature continuous.
- Check continuity labels how a body's faces meet across each shared edge.
- Create sketch works in a surface body, and a tool stays dimmed until it has what it builds from.
- Surface bodies export to STEP.

### Selection
- Ctrl+click picks several faces of a body, and edges with them; the status bar counts what is picked.
- Offset, Move and Delete faces, Thickness, fillets and chamfers by faces, Draft, Appearance and Surface texture take every picked face.

### Sketcher
- A closed sketch shades its regions outside edit mode too.

### Assistant
- The agent's replies show as formatted text, with a copy button on each message and code block.
- The chat header jumps between your own messages, and Up recalls the last one you sent.

### Workbench packages
- A double click on a package's feature opens it in its package.
- A package can remove a body it made, so cancelling a new part leaves no empty body behind.

## 0.4.0

### Workbench packages
- The workbench store: Preferences › Workbench packages › Browse lists the packages of every store you keep, with a word of care about third-party software, and installs one with a click.
- Stores have a tab of their own: printCAD's store comes first and is removable, and any other can be added by its address.

### Rebuilds
- Editing a feature rebuilds from that feature on, not the whole history, and an edit that makes the same solid as before stops there.
- Faces an edit left alone keep their meshes and outlines, and bodies build side by side.
- While a value is dragged the body meshes coarse and only the latest shape builds; it turns fine once you stop.

### Import and repair
- Solids with internal voids read, repair and export as they should: a void written the wrong way out reads as a void, repair keeps it in place, and STEP export keeps it.
- Replace shape from a STEP or IGES file takes the file's exact solid.
- PLY, glTF and VRML meshes offer Convert to solid.
- A converted solid still in facets says so, and a feature failing on it says to refine the body first.
- Repair says when it turns faces or widens tolerances.

### Sketcher
- An ellipse's minor radius dragged or set past the major becomes the major.
- A selected corner relates the constraints of every curve meeting there.

### Design
- A pad's or pocket's custom direction takes formulas.

### Interface
- The view's toolbar, cards and cube draw under dialogs and menus.
- The navigation style setting is gone; it had one style to offer.

## 0.3.0

### Import and repair
- Imported and converted solids take features: their shape becomes the body's Base, and Replace shape reads it again from another file with the features building on it.
- Convert to solid is quick and keeps curved areas as facets; Refine shape then finds their cylinders, cones, spheres and tori.
- Delete faces, Offset faces and Move faces edit an imported solid directly, the neighbouring faces following.
- Recognize holes turns a solid's round holes back into Hole features you can resize.

### Design
- Part Design is now called Design.
- Pad and Pocket push or press a picked flat face, no sketch needed.
- Surface textures: knurls, ribs, dots, hexagons and more, or a picture, pressed into chosen faces, shown in the view and written into STL and 3MF exports.
- Features drag to another place in their history in the tree.
- Curved solids draw at a smoothness you choose, finer by default.

### Sketcher
- A line dragged from its end becomes a tangent arc.
- Closed regions shade and loose ends are ringed while you edit; a profile is the sketch's closed loops, stray lines left out.
- A face picked for external geometry brings every edge around it, splines come in exact, and projected geometry draws dashed while it only guides.
- A reference picture on the sketch plane to draw over.
- Editing a sketch can cut the view at its plane (Section view, per sketch or in Preferences).
- Closing a sketch leaves it selected, ready for a feature.

### View
- A pick filter for faces, edges or whole bodies, and a double click on an edge selects the edges running smoothly on from it.
- Look at turns the view square to a face; the tree row under the pointer lights up what it stands for.
- File › Save view as picture writes the view as drawn to a PNG.

### Documents
- Autosave keeps a copy of edited documents, and the start page offers it back after a crash.
- Drop a file on the window to open or import it; Ctrl+Shift+T reopens the last tab closed.
- Cancelling a feature puts its body back exactly as its history has it.

### Interface
- Toolbar groups drag to where you want them and stay there; menus fit the window.
- Shift+Space repeats the last tool, and the command palette lists what you ran lately first.
- The status bar opens the log, the console and the assistant, and the pointer shows a busy arrow while work runs.
- Help › Check for updates, and a notice when a newer release is out.
- Number fields keep the value they show; offsets take zero and negative values.

### Assistant
- Messages sent while the agent is working wait in a queue you can edit.

### Scripting
- `doc.set_textures`, `doc.refine`, `doc.replace_shape`, `design.recognize_holes`, and `design.*` names for the Design workbench (`part.*` still works).

## 0.2.0

### Sketcher
- Text: a string in a font, size and spacing, as outlines that pad and pocket like any profile, standing on a point constraints can hold.
- Splines can be weighted; Convert to B-spline turns lines, arcs, circles, ellipses and conics into exact splines; degree, knots and weights are edited in the panel, with curvature combs to check the shape.
- Trim, extend and split work on ellipses, parabolas, hyperbolas and splines, and every curve stops at the ones it crosses.
- Fillet and chamfer between any two lines, arcs or circles, even ones that do not meet.
- Offset rounds corners, copies to both sides, replaces the original or stays linked to it by one dimension.
- Linked copies and arrays keep the originals' size and are spaced by one editable pitch.
- A sketch attaches by a mode (on a face, through three points, square to an edge…) as a datum does, and one on a datum can shift across it and turn on it.
- External geometry from another sketch or a datum, followed when they change, and external geometry that counts in the profile.
- Constraints on every kind of curve, smooth joins, an ellipse's radii and a spline's or conic's length as dimensions, and more constraints.
- Repair a sketch that has lost points, and set how far the solver goes.
- Rounded rectangles, slots and arc slots keep their tangents and equal radii when dragged.
- A negative horizontal or vertical distance puts the point the other way.
- Generators: internal ring gears with addendum and dedendum settings, and a shaft's loads with its stresses and deflection.

### Part Design
- Drag handles in the view set a pad's or pocket's length and a fillet's or chamfer's size.
- Duplicate, copy and paste features, and move a feature to another body.
- Pads and pockets stop on planes, run along a datum, a sketch line or the body's axes, start away from their profile, and take a borrowed face as their profile.
- Lofts go to a point, through several sections to one, or from faces of the solid; pipes run along edges of the solid or from a face, close to a point, and can keep their section fixed in space; a ruled loft joins unlike shapes.
- Revolutions turn about the body's own axes, and a helix about its sketch's normal.
- Datums attach in more ways: on another datum, square to a face, along an edge, through a line and a point, where a line meets a plane; and tilt.
- Borrowed geometry can be moved, fill closed edges into a face, or lend a whole solid.
- Mirror, draft and boolean take datum and edge references; thickness can grow both sides; one boolean takes several tools.
- Holes: a clearance of your own, more screw seats, and thread lengths.
- Primitives attach like datums; ellipsoids can be cut; prisms can lean.
- A through-all pocket centred on its sketch cuts both ways.
- Fillets on closed rims, and full-turn revolutions and grooves that fuse or cut cleanly.

### Assembly
- More joints: ball, universal, pin in a slot, along a path, cam and follower, and a tab centred in a slot, with point anchors and an offset on each end.
- Joints go to the origin's planes and axes and to datums, follow the faces they were picked on, can change kind and faces, and are listed under the body they hold to.
- Components: bodies grouped in the tree, nested, rigid (moving as one) or flexible (their joints live).
- Parts from other printCAD files, linked: marked when the file changes, reloaded or opened from the tree.
- Linked copies of a body, in a row, turned about an axis or mirrored, placed by dragging; replace a body and keep its joints.
- Rigid groups, redundant-joint report, and one conflicting joint no longer holds up the rest; a joint that cannot hold is not made.
- Motion over time: joints driven by formulas of t, with traces, speeds and plots, and a check for collisions along a motion.
- Saved states to return to, stepped exploded views kept in the document, and move arrows and rings on a body.
- Clearance check, mass and centre of mass, and interference checks that work at any turn.
- A numbered parts list kept in the document, with bought parts and columns of your own, listed by component if you like.
- Turning either of two coupled hinges turns both; a slider turns its body onto its axis.

### Documents
- STEP export writes several bodies as an assembly of shared parts.
- Configurations can leave bodies out.
- Record animations as GIF or numbered PNG frames as well as animated PNG.

### Interface
- Right-click menus on tree rows and bodies: Appearance, Placement, material, face colours, freeze, make unselectable, linked copy, move in history, copy formulas and more.
- Appearance, Placement and Move after open in the task panel and show every change as you make it; OK keeps it, Cancel puts it back.
- A palette of colours plus your own kept colours; single faces can have their own colour.
- A material gives a body its mass.
- Measure edges, faces, radii, areas and angles.
- Isolate a body and show every body again.
- F2 renames and Alt+Enter shows the properties of the selected tree row.
- An application icon, and a Linux install script in the download.

### Scripting
- `doc.set_body`, `doc.set_face_color`, `doc.linked_copy`, `doc.move_after` and `doc.recompute`.
- Datum references work either way, a thread can be given as `"M6"`, and `{}` stands for an empty table of settings.
- `sketch.rect` and `sketch.polyline` hold level and upright sides.

## 0.1.0

### Documents
- Several documents open at once, one per tab, each with its own undo history and camera.
- A document is served by its own local document server; saving never holds the window.
- Export the solids to STEP, STL or 3MF from the File menu; the mesh formats use the tessellation tolerance you choose.
- STEP can also go out with every surface a NURBS, for tools that read no other kind.
- Send to slicer (Ctrl+P) opens every visible body in your slicer, set in Preferences › 3D printing.
- Recent documents show a rendered preview, saved with the document.

### Sketcher
- Lines, polylines with tangent arcs, rectangles, polygons, circles, arcs, ellipses (by centre or three points), arcs of ellipses, B-splines and slots.
- Geometric and dimensional constraints, solved live, with a message for every conflicting or redundant one.
- A new dimension asks for its value by its label: type a number and press Enter, click a variable to have it read that, or save the value as a new variable on the spot.
- Trim, extend, split, fillet, offset, mirror, move, rotate, scale and arrays.
- Carbon copy brings another sketch's geometry in; merge makes one sketch of several.
- File › Import reads a DXF drawing into a new sketch in the drawing's own units: its lines, arcs, circles, ellipses and polylines (bulges as arcs) as sketch curves, splines as lines along them, hidden ones as construction, with ends that meet joined so outlines close.
- External geometry projects a solid's edges into the sketch as fixed references to constrain against, kept up to date when the solid changes.
- Rendering order puts construction or normal geometry on top.
- Drawing snaps to the origin and the two axes as it does to drawn geometry, and pins the new point there.
- Undo inside a sketch takes back the last thing done, not the whole editing session.
- Sketch edits keep the shape: moved, turned, scaled or mirrored geometry stays where it was put, copies keep their constraints, fillets stay tangent, trimmed and split lines keep their line, dragged points stay under the cursor.
- Polygons, slots and centred rectangles are held in shape as they are drawn: a polygon regular on its circle, a slot's sides tangent to its caps, a centred rectangle symmetric about its centre.
- Clicks that do nothing say why, overlapping constraint icons spread out to be clickable, and every preview matches what the click makes.
- Snapping works the same in every drawing tool and every click: to endpoints, centres, the origin, crossings, the middles of lines, square to a line or touching a circle from the last point, curves and axes, and level or plumb with the last point. Each has its own marker and name at the cursor, the click lands exactly where the marker is, and the point stays there by a matching constraint.
- Check wall thickness draws the profile's medial axis, marks where walls are thinner than the minimum set in Preferences › Sketcher, and labels the thinnest one.
- Rectangles from three corners, or from the centre and two corners, and a frame with its wall in one step.
- B-splines of degree 2 to 5, drawn through the clicked points or on them; a chain of curves joined into one spline; arcs of parabolas and hyperbolas.
- Dragging with Trim trims every curve the pointer crosses.
- Arc length, the gap between two curves, radius or diameter by kind, the angle two curves make where they meet, and refraction; an option to take away the older constraints a new one makes redundant.

- Show internal geometry adds an ellipse's axes and foci, a conic's axis and focus, or a spline's control polygon, tied to the curve.
- Intersection references: the curves where picked faces cross the sketch plane, kept up to date.
- Section view, kept per sketch, cuts away everything in front of the sketch plane while editing.
- Constraint symbols park on a second layer, and the constraint list filters by kind, name, reference, selection and relation.
- Remove axis alignment turns horizontal and vertical constraints into parallel and perpendicular ones so a group turns as a whole.

### Part Design
- The model tree is the body's history in order. Selecting a feature shows the body as it stood there, and a feature made then goes in at that point; selecting the last feature shows everything again. Moving through history is not an undo step. Going back to a point already seen shows it at once.
- A click in the tree selects; a double click opens the feature's settings. A feature stays selected after its settings close.
- A feature's profile is changed in the property panel's Inputs group, which keeps the settings panel to the operation itself.
- Fillets, chamfers, up-to-face stops, datums and sketches placed on a face (their body's own, or one another body lends) keep hold of the faces and edges they were made on when dimensions change or features go in earlier: every face is named by how it was made, and a rebuild finds it by its name.
- Pad, pocket, revolution, groove, loft, pipe, helix and primitives, additive and subtractive, and booleans between bodies.
- Holes to standard sizes, fillets and chamfers on picked edges, draft, thickness, and linear, polar and mirrored patterns.
- Datum points, lines and planes, and local coordinate systems whose planes carry sketches.
- A body's volume, surface area and centre of mass, exact wherever its faces have a closed form.
- While a feature is edited, what it adds or cuts shows see-through in its own colour over the body without it, set in Preferences › Display.
- Centre line measures a tube-like solid between two of its faces: the path through the middle of its sections, drawn over the body with its length.
- Pad and Pocket stop at the first of several faces, run along a custom direction or a picked edge, take their own end condition on each side, and make a picked flat face their profile with no sketch.
- Revolution and Groove stop to the first, to the last or up to a face, and turn about a sketch line, a datum line or a picked edge.
- Holes to ISO metric coarse and fine, unified inch, Whitworth and pipe thread standards, with class and hand; spotfaces, counterdrills and screw seats; angled drill points and tapered walls. See the Hole guide.
- Generators make an involute gear, a chain sprocket or a stepped shaft from their numbers, as a sketch that rebuilds when a number changes.
- Datums attach tangent to a face, through three points, square to or along an edge, through two points, where two planes meet, at a circle's centre, and at a body's centre of mass and inertia axes; one picked on its body's solid follows it.
- Borrow takes another body's sketch, faces or edges into a body, live or frozen: a hole through two bodies, one master sketch for several, a pad up to another body's face.
- Patterns run along a picked edge, a datum line or a sketch's axis, with uneven spacing or a step angle; fillets and chamfers follow tangent edges.
- Pipes take an orientation (standard, Frenet, a guide path, a binormal), a corner mode (transformed, right or round) and extra sections the shape changes through; helices grow per turn, a helix of no height is a flat spiral, and a subtractive helix can keep what is inside it.
- Revolution and Groove stop on any face, curved or flat, and Thickness joins its walls sharp or rounded.

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
- Parts draw in a neutral slate grey and the selection paint is the interface's blue, so white and green sketch lines, dark edges and the selection all stand apart; sketch lines being edited carry a thin dark rim that keeps them readable over any face.
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
