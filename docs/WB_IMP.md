# Workbench, Body, Document, and Sketch Editing Behavior (Generic Model)

This document describes, by topic, how a “workbench” system (similar to
FreeCAD’s PartDesign + Sketcher) typically works. It is written generically so
it can be adapted to a different application.

---

## 0. Documents, Bodies, and Basic Preconditions

### 0.1 Document and Body Relationship

- A **document** is the top‑level container for the model.
- A **body** is the main modeling container inside a document:
  - It holds sketches and operations (features) that belong to a single solid.
  - It appears as a main node in the document tree.

### 0.2 Creating and Selecting a Body

- For any modeling workbench to operate properly, there must be an **active
  body**.

- Behavior:

  - If there is **no open document and no body**, clicking the **“New Body”**
    button:
    - Creates a new document.
    - Creates a new body inside that document.
    - Adds the body to the document tree.
    - Selects that new body in the tree.
  - If there is an open document but no body, “New Body”:
    - Creates a new body in the current document.
    - Adds it to the tree.
    - Selects it.

- The **currently active body** is:
  - The body that is selected in the tree.
  - The default target for new sketches and part design operations.

### 0.3 Tree Selection and Main Stage Rendering

- The main 3D view (“main stage”) renders:

  - The content of the **currently selected item** in the tree, typically:
    - The body itself, or
    - One of the operations/features inside that body (e.g., a pad, pocket, etc.).

- Changing what is shown / active:
  - The user **double‑clicks** an item in the tree (body or operation).
  - The selected item becomes the active object, and its state is rendered as
    the main model in the 3D view.

---

## 1. Workbench Concept

- A _workbench_ is a mode or context within the application that:

  - Provides a specific toolset (commands, icons, menus) for one type of task.
  - Defines how the user interacts with objects of a certain type (e.g.,
    bodies, sketches, part features).
  - May change the visible UI panels, toolbars, and available commands.

- Preconditions:

  - Workbenches that create or edit solid geometry (e.g., PartDesign,
    Sketcher) assume:
    - There is at least one **document**.
    - There is at least one **body**.
    - A body is **selected** in the tree (or a feature within that body).

- Switching to a workbench:
  - Activates its toolbars and menus.
  - Does not necessarily change the active object, but may change how you can
    edit it.

---

## 2. Entering the Sketch Workbench

- The Sketch workbench can be entered in multiple ways:

  - Manually selecting the Sketch workbench from a global workbench selector.
  - Indirectly, by starting a **“Create Sketch”** command from another
    workbench (e.g., PartDesign), which then activates the Sketch workbench for
    editing.

- Preconditions for using Sketch tools:

  - A body (or an appropriate feature) must be available and selected.
  - If no document/body exists, the user must create one (e.g., via “New Body”
    which creates a new document and body).

- Switching to the Sketch workbench alone does **not** automatically start
  editing any particular sketch:
  - The user is in the “Sketch environment” but not in “Sketch editing mode”
    yet.
  - The tree and 3D view still show the currently selected item (body or
    feature).

---

## 3. Creating a New Sketch

- Command: **“Create New Sketch”** (button/menu).

### 3.1 Where “Create Sketch” Can Be Applied

- The “Create Sketch” command is only valid when:
  - The currently selected tree item is:
    - The **main body** (in which case a generic plane selector is used), or
    - A **supported feature** in that body that has planar faces (e.g., an
      extrusion, pad, etc.).
- It cannot be applied to arbitrary, unsupported tree items.

### 3.2 Creation Flow

1. **Support selection**:

   - If a planar face on a suitable feature is selected in the 3D view:
     - That face is used as the sketch support.
   - Otherwise, if the body (or a non‑face item) is selected:
     - A **generic plane selector** dialog is presented:
       - User chooses a base plane (XY, YZ, XZ) or a custom plane.

2. **Sketch object creation**:

   - A new sketch object is created in the active body.
   - It is inserted into the document tree, typically under the body and after
     the last feature.

3. **Tree selection update**:

   - After the new sketch is created and added to the tree:
     - The selection in the tree moves from the previously selected item
       (body or feature) to the **newly created sketch**.
     - This behavior should also apply analogously to new part design
       operations (pads, pockets, etc.): after an operation is created, the
       tree selection moves to that new operation.

4. **Entering Sketch Editing Mode**:
   - The application immediately enters **Sketch Editing Mode** for this new
     sketch:
     - The new sketch becomes the active object.
     - The 3D view reorients to the sketch plane.
     - Other objects may be dimmed or hidden, depending on settings.

---

## 4. Opening an Existing Sketch (Tree Interaction)

- In the document tree, each sketch appears as an item with a name
  (e.g., `Sketch001`).

- **Double‑click behavior on sketches**:
  - Double‑clicking a sketch tree item:
    - Ends editing of any currently open sketch or other editable object (if
      one is being edited).
    - Sets the clicked sketch as the active object.
    - Selects it in the tree.
    - Enters **Sketch Editing Mode** for that sketch.
  - Result:
    - The same editing tools used for a newly created sketch become available
      for the existing sketch.
    - The view is oriented to the sketch plane if applicable.

---

## 5. Sketch Editing Mode

- Entry conditions:

  - A new sketch has just been created via “Create Sketch”.
  - An existing sketch is double‑clicked in the tree.
  - An “Edit Sketch” command is invoked from a context menu or toolbar.

- UI changes:

  - A specific “Sketch tools” toolbar becomes active (lines, arcs, constraints,
    etc.).
  - Constraint tools, dimensional input fields, and validation tools are
    available.
  - A clear way to exit this mode is shown (e.g., “Close” or “Finish”).

- Behavior within this mode:
  - Mouse/keyboard actions are interpreted as operations on 2D geometry in the
    sketch plane.
  - New entities (lines, circles, arcs, points, etc.) are created and stored as
    elements of the sketch object.
  - Constraints (coincident, horizontal, vertical, equal, dimensional, etc.)
    are created and associated with those entities.
  - The model may update live, or on exit, so that dependent 3D features
    recompute.

---

## 6. Exiting Sketch Editing Mode

- Triggered by:

  - A “Close” / “Finish editing” command.
  - Double‑clicking another object in the tree (body, another feature, etc.).
  - Selecting a different object and invoking its own “Edit” command.

- On exit:
  - The sketch is left in its final edited state.
  - The active workbench may remain Sketcher or revert to the previous
    workbench (application design choice).
  - The 3D view returns to normal navigation and rendering of the currently
    selected item (body or feature).
  - 3D features that depend on this sketch update and recompute.

---

## 7. Relationship Between Workbench, Tree, Mode, and Body

There are four related concepts:

1. **Active Workbench**

   - Determines which toolbars/commands are visible.
   - Example: `"Sketcher"`, `"PartDesign"`, `"Assembly"`.

2. **Active Document**

   - The current open document containing bodies and features.

3. **Active Body / Active Tree Item**

   - Active body:
     - The main modeling container where features and sketches are added.
   - Active tree item:
     - The currently selected object in the tree
       (body, sketch, or part design operation).
   - The 3D view renders the state associated with the active tree item.

4. **Editing Mode**
   - A specialized state for editing a particular object.
   - Example modes: `NONE`, `EDIT_SKETCH`, `EDIT_FEATURE`, etc.

### 7.1 Typical Flows

- **Flow A: New Body and First Sketch**

  1. User has no open document/body.
  2. User clicks “New Body”:
     - New document is created.
     - New body is created and selected in the tree.
  3. User calls “Create Sketch”:
     - Plane/face selection as needed.
     - New sketch created under the body.
     - Tree selection moves to the new sketch.
     - Sketch Editing Mode starts.

- **Flow B: Create New Sketch on Existing Body/Feature**

  1. User has a document with a body and some features.
  2. User selects either:
     - The body (for a base‑plane sketch), or
     - A suitable feature with a planar face (for a face‑based sketch).
  3. User calls “Create Sketch”:
     - New sketch created and inserted into the body’s feature list.
     - Tree selection changes from the previous item to the new sketch.
     - Sketch Editing Mode starts.

- **Flow C: Edit Existing Sketch**

  1. User is in any workbench.
  2. User double‑clicks a sketch in the tree.
  3. Application:
     - Ends editing of any current object if needed.
     - Sets the clicked sketch as active.
     - Switches/ensures `currentWorkbench = "Sketcher"`.
     - Enters Sketch Editing Mode.

- **Flow D: Switch Active Operation / Feature**
  1. User double‑clicks another operation or the body in the tree.
  2. Application:
     - Exits current edit mode (if any).
     - Changes selection to the new item.
     - Renders that item’s result as the main model in the 3D view.

---

## 8. Tool Invocation and State Handling (for Implementation)

- Commands are context-sensitive:

  - **New Body**

    - If no document:
      - Creates document + body.
      - Selects the new body.
    - If document exists:
      - Creates a new body in that document.
      - Selects the new body.

  - **Create Sketch**

    - Requires:
      - A valid tree item (body or supported feature) selected.
    - Actions:
      - Determine support (selected face or plane dialog).
      - Create sketch under the active body.
      - Insert into tree.
      - **Update selection to the new sketch.**
      - Set:
        - `currentWorkbench = "Sketcher"`
        - `activeObject = newSketch`
        - `editMode = EDIT_SKETCH`

  - **Create Part Design Operation** (e.g. pad, pocket)

    - Requires:
      - A body with appropriate input (e.g., a sketch).
    - Actions:
      - Create operation as a feature under the body.
      - Insert into tree after previous feature.
      - **Update selection to the new operation.**
      - Optionally enter a dedicated edit mode for that operation.

  - **Edit Sketch / Edit Feature (double‑click in tree)**
    - If `editMode != NONE`:
      - Finish editing current object (commit/cancel).
    - Set:
      - `activeObject = clickedItem`
      - `currentWorkbench` as appropriate (e.g., `"Sketcher"` for sketches).
      - `editMode` to corresponding mode (e.g., `EDIT_SKETCH`).

- Suggested internal state:

  - `currentDocument: Document | null`
  - `currentWorkbench: string`
  - `activeBody: Body | null`
  - `activeObject: Object | null` (sketch, feature, etc.)
  - `editMode: enum { NONE, EDIT_SKETCH, EDIT_FEATURE, ... }`
  - `selection: SelectionState` (tree + 3D selection)

- View behavior:

  - When tree selection changes:
    - 3D view updates to render the selected body or operation.
  - When entering `EDIT_SKETCH`:
    - Orient camera to the sketch plane.
    - Enable 2D grid and snapping.
  - When leaving `EDIT_SKETCH`:
    - Restore previous 3D view / visibility settings.

---
