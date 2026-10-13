# Editing workflow

How documents, bodies, sketches and features behave in the app.

## Documents and tabs

- Each tab holds one document. **New** reuses the current tab if it is
  untouched, and opens a new tab otherwise.
- A new document is empty and opens in Design.
- The start page's New cards also create a first body. The "Empty sketch"
  card then opens an XY sketch in it, and the example cards load a ready-made
  model.

## The active body

- Clicking a body in the tree makes it the active body.
- Clicking a feature makes it the active object. Tools then work on that
  feature's body. Ctrl+click adds more features to the tree's selection,
  or takes one out; the row menu's Suppress then acts on all of them as
  one undo step.
- Clicking a solid in the viewport selects the face under the cursor; near
  an outline it selects the whole edge instead. Ctrl+click adds or removes
  faces of the selected body, and edges; the status bar counts them ("3
  faces, 2 edges"). A double click selects the whole body (one part of an
  assembly). The face or edge under the cursor is highlighted as you hover.

Tools are enabled only when their input exists:

| Tools | Need |
| --- | --- |
| New sketch, primitives, datums | A body |
| Pad | A sketch, or a flat face of the solid |
| Revolution, loft, pipe, helix | A sketch |
| Pocket | A solid, and a sketch or a flat face of it |
| Groove, hole | A sketch and an existing solid |
| Fillet, chamfer, draft, thickness, face tools, patterns, booleans | A solid |

A Design feature added to a surface body, or to an imported body that cannot
take features (a mesh not yet converted, a part linked from another file),
goes into a new body, and the log says so. Surface features go on a surface
body, or on one holding only sketches and datums; elsewhere they start a
new body ([Surfaces](SURFACES.md)).

## Sketches

**Creating one.** Design's New sketch switches to the Sketcher and shows
a plane picker:

- the face selected in the viewport, if any (the sketch then follows the
  face as the solid changes)
- the base planes XY, XZ and YZ, from their buttons or by clicking one of
  the origin's planes, which the view shows while the picker is open
- a plane placed by an attachment mode on what is selected on the body, as
  a datum plane is
- the body's datum planes
- the XY, XZ and YZ planes of each local coordinate system in the body

The sketch is added to the body, opened for editing, and the camera turns
square to its plane.

![A sketch being edited: a rectangle and a circle with their dimensions](images/sketch-view.png)

**Editing one.** Double clicking a sketch in the tree opens it the same way.

**While editing,** the view stays square to the sketch plane: pan, zoom and
roll work, orbit and standard views do not.

**Constraining to the model.** A constraint or dimension tool pressed
while the selection does not fit it yet stays armed (its button lit) and
takes more picks: a click on the sketch adds an element, and a click on an
edge of a solid adds that edge, or the vertex at its end when the click is
there and the tool takes a point. Once two items are picked and fit, the
edges are projected into the sketch as guides (an edge the sketch already
holds is used again) and the constraint goes on: a distance from a point to
a model edge, a point coincident with a model vertex, a circle tangent to a
round edge. That is one undo step. Enter applies the tool to a single item,
Escape or a right click puts it away.

**Finishing.** Close in the task panel, or Enter, ends the edit and returns
to the workbench you came from. Escape in the view drops what is in
progress first: a typed value, a half-drawn shape, the selection.

![The sketcher's task panel: what the solver says, its settings and the constraints](images/sketch-panel.png)

## Datums

Design's datum tools add a plane, line, point or coordinate system to the
active body. The task panel's **Attached to** list places it: on a base
plane, or by a mode built from what is selected on the body. A mode whose
picks are missing is greyed, its tooltip saying what to select first.

| Mode | Select first | Places it |
| --- | --- | --- |
| On a face | A face | On the face, tangent to it where it is curved |
| Through three points, through two points | Edges or a face, if any | Through the points, the unpicked ones near the datum |
| Square to an edge, along an edge, tangent to an edge | An edge | At or along the edge |
| Where two planes meet | | On the line two planes share |
| Midway between two planes | Two faces | Halfway between parallel faces; between faces at an angle, on the plane bisecting it |
| Centre of a curve | A circular edge | At the circle's centre, square to it |
| Centre of mass | | At the body's centre of mass, on its axes of inertia |
| On a datum, another body's origin | | On that datum or origin plane |
| Square to a face | A face | Along the face's normal |
| Through a line and a point, where a line meets a plane | An edge | Through the line |
| Where two lines cross | Two edges | Where the lines pass nearest |

Each reference row in the panel swaps what the datum stands on: **Use
selected face** or **Use selected edge** takes the current pick, and a
plane row also takes a base plane or an earlier datum plane. The midway
plane's two rows each take a face, a base plane or a datum plane, so it
can stand between a face and a datum. Between faces at an angle it takes
the bisector passing between them, through the material where they are
two faces of one solid, its x-axis along the line where they meet.

References on the body's own solid follow it: when a feature before the
datum changes, the rebuild finds each face and edge again, by name, and
the datum moves with them, along with the sketches and features built on
it. The offset, turn, tilt and flip in the panel move the datum from where
its mode places it. `pc.design.datum` makes the same datums from a script
([Scripting](SCRIPTING.md)).

## Design features

1. A tool such as Pad adds the feature, hides the sketch it uses, and opens
   the feature's task panel.
2. Changes in the panel apply to the model as you make them.
3. **OK** keeps the feature. **Cancel** removes a new feature, or restores an
   existing one to how it was when the panel opened.

Everything done in one task panel is one undo step. While the panel is
open, the material the feature adds or takes away is drawn as a translucent
preview.

![A pad's preview, drawn see-through](images/pad-preview.png)
![The Pad task panel](images/pad-task.png)

**View handles.** While its task is open, a feature shows a dot at the
end of each number it can show in the view: a pad's or pocket's length
(and the second side's), a hole's depth, a revolution's or groove's
angles on an arc round its axis, a draft's angle, a primitive's sizes, a
fillet's radius or a chamfer's size. One more dot sets where it starts: a
pad's, pocket's or hole's start offset, from its profile or from the
plane or face it starts on, and a revolution's or groove's start angle;
those go either side of zero. Drag a dot to set the number: in
steps of 1 mm or 5°, or with Shift held in fine steps of 0.1 mm or 1°
(Preferences › Design sets all four). The number shows beside the dot and
the status bar names it with its steps while it is held. A pad's,
pocket's, hole's, revolution's or groove's dot turns the feature the
other way when clicked, or when dragged back through its start. A number
a formula sets has no dot.

**Regions.** A sketch's curves may cross, meet three or more at a point,
or leave a spur: every closed area between them is a region (two
overlapping circles make three, the lens and two crescents). By default a
feature takes them all, a loop inside another loop being a hole. A pad,
pocket, revolution or groove can take only some: with its task open, click
a region on the sketch's plane to take it alone, Ctrl+click to add or take
away one; the regions left out show shaded. The task panel lists the same
regions as checks. A region keeps its name while the sketch's dimensions
change; `pc.sketch.regions` lists them for scripts, and the feature's
`regions` field holds the names it takes.

**Imported and converted solids take features too.** The first feature
added to one (a pocket, a fillet) gives the body a **Base shape** feature
first: the imported solid, kept as what the body's history starts from.
The body stays one body; its history reads Base, then the feature. Cancel
on that first feature, or deleting the Base once nothing follows it,
makes the body the plain imported solid again. Repair shape on such a
body mends its base, and the features build again on it.

**Replace shape…** (a body's menu, in the tree or the view) reads the
shape from another file: its first solid becomes the base, and the
features after it build again on the new shape, finding their faces by
name. On a body without features it simply replaces the shape. It clears
the undo history, as an import does.

**Fillet and chamfer** work on the picked edges, or with Follow tangent
edges on every edge that runs on smoothly from them (the round of a
filleted corner and the sides past it). A chamfer by two distances, or by
a distance and an angle, measures its first distance on one face of the
edge (Flip direction takes the other) and keeps it on that side all along
a tangent chain, even where the faces change from a flat side to a round
and on.

![An angle bracket with the fillet in its inside corner painted](images/bracket-fillet.png)

**Delete faces** takes picked faces out of the solid and closes each
opening from the faces around it: a bore, a boss or a round taken away,
on any solid, an imported one included. What the kernel does not close
yet it refuses by name, on the feature.

**Offset faces** pushes or pulls picked faces along their outward
normals (negative into the material; a bore offset in widens), and
**Move faces** shifts them, or turns them about an axis; in both the faces
around them follow on their own surfaces. The distance, the move and the
angle take formulas.

**Project sketch onto faces** carries a sketch along its plane's normal
onto picked faces of the solid, flat or curved: a label or a logo on a
cylinder's wall. Select the sketch in the tree (and the faces, or add them
in the task), then choose the mode: Split lines divides the faces along
the sketch's curves and leaves the solid as it is (faces later features
and colours can take), Emboss raises the regions the curves enclose out of
the faces by the height, and Deboss sinks them in. Put the sketch beside
the faces rather than through the solid: each curve lands on the side
nearest the sketch. Carried straight onto a round wall, the sketch keeps
its look from in front, but on the wall its widths stretch toward the
sides.

Tick **Wrap onto a cylinder or cone** to unroll the sketch onto the
surface instead, as a paper label goes on a can: every length in the
sketch is the same length on the wall. The sketch's middle lands where
the straight projection would put it; from there its width runs round the
axis and its height along it (on a cone, along the slope). A rectangle as
wide as the wall's circumference goes all the way round, so a band or a
scale can wrap much further than a straight projection reaches. The
picked faces must all lie on one cylinder or cone, and the sketch's plane
must run along the axis, not across it; a sketch that would reach more
than once round is refused.

**Recognize holes** reads the body's solid for round holes: full bores
open at one end or both, ending flat or in a drill point. It deletes
their faces (one Delete faces feature) and drills them again as Hole
features, one per set of alike holes on one plane, from a hidden sketch
of their centres, so a recognized hole's diameter, depth and point are
numbers to change like any hole's. Bores it cannot describe (a
counterbore, a countersink, a slot) are left as they are and counted in
the log. On an imported solid it gives the body its base shape first.

**Patterns** repeat features (the selected one, else the body's last
that adds or cuts): **Mirrored** across a plane, **Linear pattern** along
a line, **Polar pattern** about an axis, **Circular pattern** in
concentric rings (each ring stepped out from the axis by the ring spacing,
with a count of its own), **Point pattern** at a sketch's points (a copy at
each point after the first, moved by how far it is from the first, so the
original belongs on the first point drawn), and **Path pattern** along a
sketch's chain of curves or an edge of the solid. A path pattern spreads
its occurrences over the whole path (evenly round a closed one) or spaces
them a distance apart along it; the original stands for the path's start,
or its end with From the path's end, and each copy moves from there to its
place, turned as the path turns with Turn with the path. An edge is found
again on the solid as it stands where the pattern does in the history, so
the copies follow the edge when the features before it change. **Multi
transform** chains steps of every kind, a path among them, each applied to
every occurrence before it. With a pattern's task open, a dot stands at
each occurrence; click one to leave it out or bring it back.

**Generators** (the toolbar's Generators list) make a sketch from a few
numbers, set in its panel or with formulas: an involute gear to pad, a
chain sprocket to pad, a stepped shaft to revolve, and a shaft's keyway
to pocket. Each asks for its plane as a new sketch does.

![A gear with a keyed bore and a chain sprocket, each a generated sketch padded](images/generators.png)

- A gear's root is the one a rack cutter leaves, its rounded tip tracing
  the root fillet; on a pinion of few teeth (below about 17 at 20°) the
  cutter undercuts the flanks above the base circle, and the panel says
  so. Root as the cutter leaves it off draws a root fillet arc instead
  (what gears made before had).
- Keyway in the bore cuts a parallel key's keyway into a gear's or a
  sprocket's bore. Its width and depth at 0 take the standard key's by
  the bore (DIN 6885, ISO 773, bores of 6 to 230 mm).
- The Keyway generator draws the key's slot, round ended or square, its
  width the standard key's for the shaft's diameter; the panel gives the
  depth to pocket it. Placed on the round face of a shaft, it runs along
  the shaft's axis.

**Borrowed geometry** brings another body's sketch, or faces and edges of
its solid, into this body, where the two bodies sit (Borrow geometry in Design;
`design.borrow`). It follows its source, or keeps a frozen copy. A
borrowed face is:

- a pad's or pocket's profile, when flat
- the face a pad or pocket stops on (Up to its name in the Type list)
- one of the faces an Up to shape side stops on (its Borrowed stop faces
  ticks), beside picked faces of the body's own solid; with only
  borrowed faces it needs no earlier material
- the face a revolution or groove turns until (Up to its name in the
  Type list), on the first feature too
- the plane of a new sketch (select the borrow, then New sketch), or of
  an existing one: a sketch's tree menu offers Map onto each flat face
  its body borrows (`design.map_sketch`). Either way the sketch follows
  the face as its body changes or moves.

A borrowed edge is an axis to turn about, a direction to pad along, or a
pipe's path.

## The tree

- Double clicking a feature opens it for editing in the workbench that owns
  it.
- The eye on a row shows or hides a feature, a body or an imported part.
- A feature's right-click menu offers edit, rename, its body's appearance
  and placement, suppress, hide, move up, down or after another, set or
  clear the tip, freeze the body, cut, copy, paste, delete, copy and paste
  formulas, recompute and properties. A body's menu, in the tree or the
  view, offers appearance, placement, freeze, make unselectable, linked copy
  and the rest. Features after the tip are shown muted and are not built.
- Selecting a feature shows its body as it stands there: the tip moves to
  it (back to the whole history when it is the last), later features are
  muted, and a feature made then goes in right after it. Moving through
  history this way is not an undo step.
- Appearance, placement and moving in history open in the task panel: the
  view shows each change at once, OK keeps it as one undo step and Cancel
  puts back what was there.
- Deleting a body removes its features. Unless the body was still empty,
  this clears the undo history.
- Array of copies… (a body's menu) repeats the body in a row (a count, a
  spacing and a direction), a ring (a count, the angle between
  neighbours and an axis), along a path (a sketch's chain of curves or a
  selected edge of a solid: the copies spread over its whole length or a
  spacing apart, measured along it, and turned with it when asked) or at
  points (a sketch's points that are no curve's end or centre; the body
  stands at the first), each repeat a linked copy the array owns. A
  component's tree menu offers it too: every body in the component is
  copied, each occurrence's copies held together in a rigid component of
  their own beside it, and deleting one of them skips the whole
  occurrence. While
  the task lays out a row, an arrow stands on its first copy: dragging it
  changes the spacing (in 0.1 mm steps, Escape puts it back). A path
  or point array reads its places when it is made or edited. The
  copies follow every change to the body and stand where the array puts
  them from where the body sits, so moving the body carries them. They
  draw, pick, export and print like any body, and the parts list counts
  them with the body. The tree gathers an array's copies under a row of
  its own (its eye hides them all, Delete takes the array away and leaves
  the body) and names each copy's place in it; Edit array on that row,
  the body or a copy changes the count, spacing, angle or axis in the
  task panel, one undo step, and deleting a copy (or Skip this copy)
  leaves that place out until Edit array brings it back.

![The feature tree of the bracket: its sketches, pad, holes and fillet in order](images/feature-tree.png)

## Rebuilding

- Changing a feature marks it dirty, along with everything that depends on
  it. Editing a sketch dirties the features built from it.
- Each frame, every body with a dirty feature is rebuilt on the kernel's
  threads. A body builds one plan at a time; changes made meanwhile wait,
  and only the latest one is built.
- A failed rebuild marks its feature in the tree and in the task panel, and
  logs the error. The body shows the history before the failure.
- Recompute All rebuilds every body.

## Undo

Undo steps back through the recorded edits by applying their inverses. Each
mouse gesture or command is one step, and the history keeps 64 steps. In a
sketch's editing session every edit (a line drawn, a constraint added) is a
step of its own. Imports, deleting a body that has anything in it, Replace
shape, converting a mesh, refining and repairing a shape clear the
history.
