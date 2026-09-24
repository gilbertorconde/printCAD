# Assembly

The Assembly workbench places bodies against each other with joints. A
joint belongs to the body it moves and names the body it holds against.
Joints solve when one is made or edited and when a body moves, and the
moves are ordinary edits: a joint and the moves it causes undo as one step.

## Making a joint

Pick a joint tool, click a face on the body to move, then a face on the
body it goes against. The first body moves; the second stays where it is.

| Joint | Takes | Leaves free |
| --- | --- | --- |
| Mate (M) | two flat faces | sliding on the face, turning about its normal |
| Align (A) | two axes | turning about the axis, sliding along it |
| Angle (N) | two flat faces | everything but the angle |
| Hinge (H) | two axes | turning about the axis |
| Slider (L) | two axes | sliding along the axis |
| Fix together (X) | any face on each | nothing: the body moves with the other |
| Parallel (R) | two flat faces | everything but the two tilts |
| Perpendicular (Shift+R) | two flat faces | everything but one tilt |
| Distance (D) | two flat faces | everything but the distance |
| Tangent (T) | a flat face and a round one | rolling and sliding on the flat face |

An axis is a round face (a hole, a pin, a boss) or an edge: a circular
edge gives its circle's axis, so a hole's rim works, and a straight edge
gives its own line. Settings a tool does not ask for start at what the
bodies make now (an angle, a distance), so making the joint moves nothing
it need not.

Ground (F) keeps a body where it is; the bodies joined to it are placed
against it. The status bar says how many motions the joints leave open,
and for the selected body which ones.

## Driving and limits

A hinge's angle and a slider's position can be driven: tick Drive in its
settings and give a value, or a formula (see [VARIABLES.md](VARIABLES.md)).
A hinge's angle counts from where it sat when the joint was made. Limits
keep the motion within a range while it is not driven. Play sweeps a
driven joint through its limits (or a whole turn, or 25 mm either way) to
show the motion, and puts it back when stopped.

## Dragging

Drag a jointed body with the left mouse button: it follows the mouse as
far as its joints let it, so a door swings on its hinge rather than
sliding off it. A grounded body, or one with no joints of its own, does
not drag; use Move body (G) for those. A drag is one undo step.

## Checking interference

Check interference (I) looks at every pair of visible solid bodies and
lists those that share material, with how much. Each clash is marked in
the view with its volume. Mesh bodies are left out; convert one to a solid
to check it.

## Exploded view and parts list

Exploded view (E) moves every body straight out from the middle of the
assembly by the spread you set. Nothing is kept: the bodies go back when
it closes.

Parts list (B) lists every part with how many there are, bodies of the
same shape counted together, and the size of each along its own axes.
Copy as CSV puts it on the clipboard for a spreadsheet.

## From a script

Every joint tool is a command (`asm.mate`, `asm.hinge`, ...) taking faces
as `pc.doc.faces` lists them. `asm.set` changes a joint's settings,
`drive` and `limits` included; `asm.travel` reads a hinge's angle or a
slider's position; `asm.freedom`, `asm.interference` and `asm.parts` read
the assembly. [SCRIPTING.md](SCRIPTING.md) lists them all.
