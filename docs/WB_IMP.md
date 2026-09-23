# Editing workflow

How documents, bodies, sketches and features behave in the app today.

## Documents and tabs

- Each tab holds one document. **New** reuses the current tab if it is
  untouched, and opens a new tab otherwise.
- A new document is empty and opens in Part Design.
- The start page's New cards also create a first body. The "Empty sketch"
  card then opens an XY sketch in it, and the example cards load a ready-made
  model.

## The active body

- Clicking a body in the tree makes it the active body.
- Clicking a feature makes it the active object. Tools then work on that
  feature's body.
- Clicking in the viewport selects a face, or the whole body on a double
  click.

Tools are enabled only when their input exists:

| Tools | Need |
| --- | --- |
| New sketch, primitives, datums | A body |
| Pad, revolution, loft, pipe, helix | A sketch |
| Pocket, groove, hole | A sketch and an existing solid |
| Fillet, chamfer, patterns, booleans | A solid |

A feature added to an imported body goes into a new body, since an imported
solid has no history to add to.

## Sketches

**Creating one.** Part Design's New sketch switches to the Sketcher and shows
a plane picker:

- the face selected in the viewport, if any
- the base planes XY, XZ and YZ
- the body's datum planes
- the XY, XZ and YZ planes of each local coordinate system in the body

The sketch is added to the body, opened for editing, and the camera turns
square to its plane.

**Editing one.** Double clicking a sketch in the tree opens it the same way.

**While editing,** the view stays square to the sketch plane: pan, zoom and
roll work, orbit and standard views do not.

**Finishing.** Close in the task panel, or Enter or Escape, ends the edit and
returns to the workbench you came from.

## Part Design features

1. A tool such as Pad adds the feature, hides the sketch it uses, and opens
   the feature's task panel.
2. Changes in the panel apply to the model as you make them.
3. **OK** keeps the feature. **Cancel** removes a new feature, or restores an
   existing one to how it was when the panel opened.

Everything done in one task panel is one undo step.

## The tree

- Double clicking a feature opens it for editing in the workbench that owns
  it.
- The eye on a row shows or hides a feature, a body or an imported part.
- A feature's right-click menu offers suppress, hide, move up or down, set
  or clear the tip, and delete. Features after the tip are shown muted and
  are not built.
- Deleting a body removes its features and clears the undo history.

## Rebuilding

- Changing a feature marks it dirty, along with everything that depends on
  it. Editing a sketch dirties the features built from it.
- Each frame, every body with a dirty feature is rebuilt on the kernel
  thread.
- A failed rebuild marks its feature in the tree and in the task panel, and
  logs the error.
- Recompute All rebuilds every body.

## Undo

Undo steps back through the recorded edits by applying their inverses. Each
mouse gesture or command is one step, and the history keeps 64 steps.
Imports, deleting a body, converting a mesh and repairing a shape clear the
history.
