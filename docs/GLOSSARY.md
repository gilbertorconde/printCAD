# Glossary

The words printCAD uses, each as the app means it, with the guide that
says more.

**Array of copies.** Linked copies of a body or a component placed along a
line, round an axis, along a path or on a sketch's points, kept as one
array that adds and removes its copies as its count changes.
([Editing](EDITING.md))

**Assembly.** The workbench that holds bodies together with joints, moves
them, checks where they overlap and lists their parts.
([Assembly](ASSEMBLY.md))

**Base solid.** An imported, converted or repaired shape a body builds on:
its first feature, after which ordinary features add to it and cut it.
([Editing](EDITING.md))

**Body.** One part: a solid (or a mesh, or sheets) with its own history of
features and its own placement in the scene.
([Editing](EDITING.md))

**Bought part.** A body the parts list marks as bought rather than printed,
which exports and the slicer leave out. ([Printing](PRINTING.md))

**Clipping plane.** A plane that hides everything on one side of it, to look
inside a part; it changes nothing. ([Camera](CAMERA.md))

**Component.** A named group of bodies in the tree. A rigid one moves as one
in an assembly; a flexible one lets its bodies move on their joints.
([Assembly](ASSEMBLY.md))

**Configuration.** A row of the configurations table: values for chosen
variables, one row active at a time, so one document makes several sizes
of a part. ([Variables](VARIABLES.md))

**Constraint.** A rule a sketch keeps: a length, an angle, two lines
parallel, a point on a curve. The sketch moves its geometry until every
rule holds. ([Sketcher](SKETCHER.md))

**Cosmetic thread.** A thread drawn on a round face and named in the parts
list and exports, without being cut into the solid. ([Holes](HOLES.md))

**Coupling.** A tie between two joints' motions: gears, a belt, a rack and
pinion, a screw. ([Assembly](ASSEMBLY.md))

**Datum.** A plane, line, point or coordinate system placed to build on,
attached to what it was made from and following it.
([Editing](EDITING.md))

**Drive.** A hinge's or slider's motion held at a value or kept between
limits, which the assembly can sweep through. ([Assembly](ASSEMBLY.md))

**Edit session.** Working inside one feature, such as a sketch, with the
view square to its plane until it is closed. ([Sketcher](SKETCHER.md))

**Faceted.** A solid converted from a mesh whose curved areas are still the
mesh's flat facets; Refine shape finds its round faces.
([Document model](DOCUMENT_MODEL.md))

**Feature.** One step of a body's history: a sketch, a pad, a hole, a
fillet. Editing a feature rebuilds everything after it.
([Editing](EDITING.md))

**Formula.** An expression that sets a number, such as `Plate.width / 2`,
worked out again whenever what it reads changes.
([Variables](VARIABLES.md))

**History.** A body's features in the order they build it, top to bottom in
the tree. ([Editing](EDITING.md))

**Joint.** A relation between two bodies in an assembly: faces mated, axes
aligned, a hinge, a slider. ([Assembly](ASSEMBLY.md))

**Kernel.** The geometry engine that builds solids from features, meshes
them for the view and reads and writes CAD files.
([Architecture](ARCHITECTURE.md))

**Linked copy.** A body that takes another body's shape and follows its
changes, placed where it stands; a mirrored copy takes it mirrored.
([Editing](EDITING.md))

**Linked part.** A body whose shape is read from another printCAD file,
refreshed when that file changes. ([Editing](EDITING.md))

**Mesh body.** A body made of triangles only, from an STL, OBJ, 3MF or
similar file. It draws and exports; Convert to solid makes it take
features. ([Editing](EDITING.md))

**Package.** A workbench someone else wrote, installed from a file or a
GitHub address and run in a sandbox with only what you allow it.
([Workbench packages](PLUGINS.md))

**Parts list.** The table of bodies an assembly is made of, with counts,
material, filament mass and what is bought. ([Assembly](ASSEMBLY.md))

**Pick.** What a click in the view takes: a face, an edge, a vertex or a
whole body. ([Editing](EDITING.md))

**Print layout.** The parts laid flat on the printer's bed, plate after
plate, for export and the slicer, without moving the bodies.
([Printing](PRINTING.md))

**Profile.** A sketch's closed areas, which a pad, pocket or revolution
takes. ([Sketcher](SKETCHER.md))

**Recipe.** A whole part written as a script, which the test suite runs.
([Scripting](SCRIPTING.md))

**Refine.** Merging the faces a feature leaves split on one surface, so the
solid has the faces it looks like it has.
([Document model](DOCUMENT_MODEL.md))

**Region.** One area of a sketch bounded by its curves, which a feature can
take on its own. ([Sketcher](SKETCHER.md))

**Repair shape.** Mending an imported shape the kernel's checker finds
broken, before features are built on it. ([Editing](EDITING.md))

**Stand-in.** A shape kept for a body whose features do not build yet, such
as an imported file's saved part, shown until they do.
([Editing](EDITING.md))

**Task.** The panel on the right while a feature or tool is being set up,
with OK and Cancel; the whole task is one undo step.
([Editing](EDITING.md))

**Tip.** The feature a body's history stops at: features after it are drawn
faded and left out of the solid, and a new feature goes in after it.
([Editing](EDITING.md))

**Variable set.** A named group of variables formulas can read.
([Variables](VARIABLES.md))

**Workbench.** A set of tools for one kind of work: Design for solid
features, the sketcher, surfaces, assembly, and any installed package.
([Editing](EDITING.md))
