# FreeCAD Part Design & Sketcher — Complete Reference

Deep-dive into how FreeCAD's two core parametric workbenches actually work, as design
input for printCAD. Current as of **July 2026** (research from wiki, blog, source code,
and PRs — sources at the end).

**Version anchors** (the "recent development boost"):

| Release | Date | Headlines |
|---|---|---|
| 0.21 | Aug 2023 | Sketcher toolbar split (edit/non-edit), Elements widget rework, snap manager, visual layers |
| **1.0** | Nov 2024 | Toponaming mitigation (element maps), Assembly WB + Ondsel solver, Sketcher UX overhaul (on-screen input, snapping, offset/transform tools), Materials system, VarSets, feature Suppress |
| 1.0.1 / 1.0.2 | May / Aug 2025 | Bugfix-only point releases |
| **1.1** | Mar 2026 | Sketcher external-geometry projection/intersection (defining by default), transparent PD previews + gizmo draggers, Hole tool redesign, core datums/LCS replace PD datums, Assembly simulation + BOM, Theme Editor, Clarify Selection |
| 1.1.1 | Apr 2026 | Bugfix (latest stable) |
| 26.3 | branches Sep 2026 | **No 1.2** — FreeCAD moved to CalVer (YY.N), 3 releases/year, weekly dev builds (FEP-0003) |

---

## 1. Big picture — how the two workbenches interlock

```
Std Part (positional container, never fuses)
 └─ PartDesign::Body            ← ONE contiguous solid (multi-solid default-on in 1.1)
     ├─ Origin                  ← XY/XZ/YZ planes, X/Y/Z axes, origin point (core datums in 1.1)
     ├─ Sketch ──────────────┐    (attached to origin plane / datum / face)
     ├─ Pad          ◄───────┘    profile consumed by feature
     ├─ Sketch ──────────────┐
     ├─ Pocket       ◄───────┘
     ├─ Fillet                  ← dress-up: previous solid + selected edges
     └─ LinearPattern  ◄─ Tip   ← Body exposes the Tip feature's shape
```

- Sketcher produces constrained 2D profiles; Part Design consumes them as feature inputs.
- The Body is a **strictly linear history**: each solid feature takes the previous
  feature's solid and produces a new one. No feature branches.
- Double-click semantics are the "jump" mechanism: feature → its task dialog,
  sketch → straight into Sketcher edit mode (§3.4).

---

## 2. Part Design — document model

### 2.1 The Body

| Element | Type | Meaning |
|---|---|---|
| Rule | — | Models a single contiguous solid; every feature must intersect existing material and is fused/cut into one solid. Multi-solid: experimental opt-in in 1.0, **default in 1.1** (`AllowCompound`) |
| `Group` | LinkList | Ordered list of everything in the Body (features, sketches, datums) = tree order |
| `Tip` | Link | Feature exposed as the Body's shape; normally the last solid feature; reassignable ("Set tip", white-arrow-in-green-circle overlay) |
| `BaseFeature` | Link | Optional external solid (import/other WB) adopted as first feature — set by selecting a solid before Create Body, or drag-drop |
| `Origin` | hidden Link | The 3 planes / 3 axes / point; toponaming-safe attachment anchors; **1.1: rebuilt on core datums → file-format break with ≤1.0** |
| `DisplayModeBody` | Enum (View) | `Through` (internals selectable) vs `Tip` (only final shape exposed — use from other WBs) |
| Active Body | UI state | Only one; bold + highlight; new features require it; auto-activated if only one exists |

Visibility rules: only **one solid feature visible at a time**; adding a feature hides
the previous one. Spacebar on an earlier feature previews that history state without
moving Tip.

### 2.2 Feature stack — how operations chain

```
          per-feature link                    per-feature tool solid
 ┌────────────────────────────┐        ┌───────────────────────────────┐
 │ BaseFeature (Link)         │        │ AddSubShape (Part shape prop) │
 │ = previous solid feature   │        │ = raw prism/revolve/sweep     │
 └─────────────┬──────────────┘        │   built from the profile      │
               │                       └───────────────┬───────────────┘
               ▼                                       ▼
        prev.Shape ────────► fuse (additive) / cut (subtractive) ────► this.Shape
                                        │
                                        └─ Refine=true → clean residual edges
```

- Every `PartDesign::Feature` has: `BaseFeature` (predecessor link), hidden `_Body`
  (owner backlink), `Suppressed` (1.0+, with `SuppressedShape` kept for restore).
- `FeatureAddSub` adds `AddSubShape` + `Type {Additive, Subtractive}`.
- **Dress-ups** (`PartDesign::DressUp`): property `Base` is a `LinkSub` = previous
  feature **plus the selected sub-elements** (edges/faces); the dressed solid replaces
  the whole previous solid.
- Deleting a feature: `Body::removeObject()` reroutes successors' `BaseFeature` links
  around it (chain repair).
- Patterns in "Transform tool shapes" mode re-apply the transformed `AddSubShape`s of
  the selected `Originals`; "Transform body" mode (1.0 default) transforms the whole
  base shape.

### 2.3 Tree mechanics

| Mechanic | Behavior |
|---|---|
| Order = recompute order | Linear BaseFeature chain ⇒ the Body's dependency graph is one branch, recomputed in tree order |
| Set tip | Features **after** tip are hidden and excluded from the shape; new objects insert after the tip; set tip back when done |
| Insert mid-tree | `insertObject()` after a given feature; insertion point = before next solid after Tip; sketches/datums are skipped when finding prev/next solid |
| Reorder | "Move object after other object" → dialog; blocked when it would invert dependencies; order changes results (pocket-before-pad ≠ pad-before-pocket) |
| Move to other body | "Move object to other body"; lands under target tip; dangling cross-body refs error |
| Drag & drop | Drag solids into a Body → becomes BaseFeature; drag sketches to re-parent |

### 2.4 Editing lifecycle & "jumping"

```
tree double-click:
  Feature ──► its task dialog reopens (Model tab locked, one dialog at a time)
  Sketch  ──► Sketcher EDIT MODE directly (even if consumed by a feature)
              · owning Body/doc auto-activated
              · visibility automation: hide dependents, show external-geometry
                sources, show attachment support, optional section view
```

- Dialog edits preview live in 3D; expensive tools (patterns, helix) have an explicit
  **Update view** checkbox.
- **1.1**: transparent live previews for all add/sub features + optional profile
  highlight + **interactive 3D gizmo draggers** (Fillet, Chamfer, Transform — drag
  values in the viewport).
- OK = commit + recompute. Cancel = revert; on a *new* feature it deletes the feature.
- **Every dialog option maps 1:1 to a Data property** — editing either recomputes.
  Some knobs are property-only (`Refine`, `SupportTransform`, `Suppressed`).

### 2.5 Recompute & error model

| State | Tree overlay | Meaning |
|---|---|---|
| Touched | white ✓ on blue | Needs recompute (property changed / upstream changed / marked manually) |
| Error | white ! on red | Recompute failed; tooltip shows the error text; downstream stalls, 3D keeps last good state |
| Suppressed | red backslash | Feature skipped; downstream recomputes as if absent (1.0+) |
| Frozen | cyan ice crystal | Excluded from recompute (`Toggle freeze`, exposed in PD in 1.1) |

- Document-wide: "Skip recomputes" context option suspends auto-recompute; disabling +
  refresh drains the queue.
- Dependencies must form a **DAG**; `Std DependencyGraph` visualizes it (red arrows =
  out-of-scope links, e.g. referencing inside a foreign Body → "Links go out of
  allowed scope").

### 2.6 Datums & the attachment engine

Types: DatumPlane, DatumLine, DatumPoint, Local Coordinate System (LCS).
**1.1 replaces PD-specific datums with core datums** shared across workbenches
(Assembly joints attach to them too).

Attachment engine (`Attacher`, also used by sketches): 4 engines —
`AttachEngine3D`, `AttachEnginePlane`, `AttachEngineLine`, `AttachEnginePoint`.

MapModes (plane/3D engine — mode ← required references):

| MapMode | References |
|---|---|
| Deactivated | none (plain Placement rules) |
| Translate origin | vertex (orientation from Placement) |
| Object's X Y Z / X Z Y / Y Z X | object (copy/permute its placement axes) |
| FlatFace ("XY on plane") | planar face |
| TangentPlane | face + vertex |
| XY parallel to plane (1.0) | plane + vertex |
| NormalToEdge | edge (+ optional vertex) |
| FrenetNB / FrenetTN / FrenetTB | curve (+ vertex or `MapPathParameter`) |
| Concentric / Revolution Section | curved edge |
| ThreePointsPlane / ThreePointsNormal | 3 vertices (or line combos) |
| Folding | 4 lines (sheet-fold) |
| InertialCS | whole shape (principal axes of inertia) |
| Align O-Z-X … O-Y-X (6 modes) | origin vertex + 2 direction refs |

Line engine adds: ThroughTwoPoints, Intersection (1.0), axis-of-curvature,
directrices, asymptotes, tangent, binormal, normal-to-surface, principal axes.
Point engine: object origin, foci, on-edge, center of curvature/mass, vertex,
proximity points.

- **Attachment Offset** = extra Placement applied *in the attachment coordinate
  system* (x/y/z translation + rotations + Flip sides = 180° about local Y);
  `MapReversed` flips orientation.
- Why datums: attaching to origin planes / datums / LCS instead of generated faces
  decouples downstream objects from topology churn (toponaming insurance).

### 2.7 ShapeBinder vs SubShapeBinder

| Aspect | ShapeBinder | SubShapeBinder |
|---|---|---|
| Parents | Exactly one object (or its subelements) | Many parents/subelements |
| Cross-document | No | Yes (Link-based) |
| Placement tracking | `TraceSupport` bool (default off) | Always; `Relative` prop |
| Sync control | live when traced | `BindMode`: Synchronized / Frozen (snapshot) / Detached |
| Extras | — | `Fuse`, `MakeFace`, `PartialLoad`, 2D offset |
| Role | Simple borrow (hole through bodies, sketch reuse) | Recommended default for cross-body/cross-doc refs, boolean tools, master-sketch distribution |

Both exist to import external geometry into a Body without scope errors.

### 2.8 Toponaming — problem and 1.0 mitigation

- **Problem**: OCCT names elements sequentially (`Face1`, `Edge3`) and renumbers on
  every recompute → sketch supports, fillet edge lists, up-to faces silently rebind or
  break after upstream edits.
- **1.0 fix** (realthunder's algorithm in core): `Part::TopoShape` carries an
  **element map** — stable mapped element names hashed from the generating operation
  history. Gives: (1) broken refs are *identified* instead of silently misbound,
  (2) suggested fixes for edge lists, (3) frequent automatic re-resolution
  (sketch-on-face survives most parametric edits).
- **Still a mitigation, not a solution**: structural changes (insert/delete/reorder
  features) can still break refs; rollout is per-workbench; manual re-pick via Map
  Mode dialog remains the fallback. Best practice unchanged: datums as supports,
  master sketches, binders, dress-ups last.

### 2.9 Multi-body

- **PartDesign Boolean** (a Body feature): Fuse/Cut/Common of *tool bodies* against
  the active Body; tools (Bodies, Clones, binders, external solids) are moved into
  the Body's Group and re-interpreted in its origin. `Display`: Result | Tools.
- Reuse: **PartDesign Clone** (parametric body copy), **Std Link** (instances,
  cross-doc), SubShapeBinder snapshots. **Std Part** groups bodies positionally.

### 2.10 What selecting things shows (tree + property editor)

| Selected | Data properties (editable unless noted) |
|---|---|
| Body | Tip, Base Feature, Group, Placement; hidden: Origin, AllowCompound. View: Display Mode Body |
| Sketch-based feature (e.g. Pad) | Type, Length/Length2, UpToFace, Offset, Direction group (UseCustomVector, Direction, ReferenceAxis, AlongSketchNormal), TaperAngle(2), Midplane, Reversed, Profile (LinkSub), AllowMultiFace, Refine, Suppressed; hidden: AddSubShape, BaseFeature, _Body; `Shape` read-only output; feature Placement hidden (Body governs) |
| Sketch | Attachment group: MapMode (with `…` button → attachment dialog), AttachmentSupport (LinkSubList), MapReversed, MapPathParameter, AttachmentOffset; Placement read-only while attached |
| Datum | MapMode, AttachmentOffset, Placement, size/resize display props |
| Anything | Context menu → "Show hidden" surfaces hidden properties |

View tab (all): Visibility, Display Mode, Shape Appearance/Color (1.0 material-based),
Line/Point width+color, Transparency, Selectable, Bounding box.

3D preselection: hover highlights + tooltip/status bar with full element path
(`Document#Body.Pad.Face6`) + cursor 3D coordinates. Selection view panel shows the
same for current selection.

---

## 3. Part Design — operations catalog

Menu groups: helpers (Body, Sketch flyout, Validate, binders, Clone, datum flyout) ·
additive · subtractive · patterns · dress-ups · Boolean · extras (InvoluteGear,
Sprocket, Shaft wizard — 2D generators meant to be Padded).

### 3.1 Additive

| Op | Input | Termination / modes | Key options |
|---|---|---|---|
| **Pad** | sketch, solid face(s), binder face | Dimension · TwoLengths · ToLast · ToFirst · UpToFace · **UpToShape** (1.0, multi-face) | Offset-to-face, Symmetric-to-plane, Reversed, direction (normal / reference edge / custom vector), taper angle(s). **1.1: two-sided mode = independent end conditions per side** (PR #21794) |
| **Revolution** | sketch/faces | Dimension (≤360°) · ToLast · ToFirst · UpToFace · TwoDimensions (all 1.0) | Axis: sketch V/H axis, construction line, base X/Y/Z, reference edge/datum line; Midplane, Reversed |
| **Additive Loft** | base profile + Add Section (2+ sections, reorderable) | — | Ruled (straight transitions), Closed (loop). Rules: matching segment counts; vertex only at start/end; no coplanar consecutive sections |
| **Additive Pipe** | profile + path (whole sketch or edge list; no branches) | Section transform: Constant · Multisection | Orientation: Standard ⊥ path / Fixed / Frenet / Auxiliary (2nd guide path + curvelinear equivalence) / Binormal vector; corner transition: Transformed / Right / Rounded |
| **Additive Helix** | sketch/face | Modes: Pitch-Height-Angle · Pitch-Turns-Angle · Height-Turns-Angle · Height-Turns-Growth (H=0 → spiral) | Axis like Revolution + sketch normal; cone angle ±89°, LeftHanded, Reversed; `HasBeenEdited=false` → auto-proposes safe pitch |
| **Primitives** ×8 | attachment dialog (full engine) | Box L/W/H · Cylinder R/H/angle/skew · Sphere R + 3 angles · Cone R1/R2/H · Torus R1/R2 + angles · Ellipsoid R1-3 · Prism N/circumradius/H/skew · Wedge min/max spans (zero top spans → pyramid) | First feature or fused; re-edit via double-click |

### 3.2 Subtractive

Same dialogs, cut instead of fuse: **Pocket** (ThroughAll = internal 10 m cut; UpToShape
in 1.0; direction opposite the normal), **Groove** (= Revolution cut, 1.0 got the full
mode set), Subtractive Loft/Pipe/Helix (helix extra: `Outside` = keep intersection
instead of cutting), subtractive primitives.

**Hole** (the standards-driven one):

- Input: circles/arcs **centers** of one sketch (radii ignored); **1.1: sketch points work too**.
- Thread profile: None / ISO metric coarse+fine / UTS coarse/fine/extra-fine /
  **1.1: BSW, BSF, BSP, NPT** (with ISO 7-1 / ASME B1.20.1 auto taper angles).
- Threaded ⇒ tap drill size; unthreaded ⇒ clearance Standard/Close/Wide (ISO 273).
- Model Thread (real helical geometry, expensive) — 26.3 dev adds **Cosmetic Thread**.
- Size (M6, 1/4-20…), Class (6H, 2B…), Direction L/R, Depth: Dimension / ThroughAll.
- Hole cut: None / Counterbore / Countersink / Counterdrill (0.21) / norm seats
  (ISO 4762 etc.) / **user-extensible via JSON** in `<AppData>/PartDesign/Hole`;
  spotface = counterbore with custom Ø/depth.
- Drill point: Flat / Angled (118°/135°) + count-toward-depth; Tapered; Reversed.
- **1.1: task panel redesigned around a live hole diagram**, irrelevant controls hidden.

### 3.3 Transformations

Common: `Originals` list; mode **Transform body** (whole solid, default) vs
**Transform tool shapes** (only selected features' AddSubShapes, order matters);
occurrences not touching the parent solid are dropped; patterns-of-patterns only via
MultiTransform; dress-ups join via `SupportTransform`.

| Op | Reference | Parameters |
|---|---|---|
| Mirrored | sketch axes, construction line, base planes, reference face/datum plane | plane only |
| LinearPattern | sketch axes/normal, construction line, base axes, reference edge/datum line, reverse | Mode (1.0): Overall Length / Offset (spacing); Occurrences. **1.1: non-uniform per-occurrence spacing; unified Linear/Polar panel** (PR #22389) |
| PolarPattern | axis list as above | Mode: Overall Angle (360° = full circle) / Offset Angle; Occurrences |
| Scaled | — (MultiTransform only) | Factor (last occurrence), Occurrences; scales about feature CoG |
| MultiTransform | combines all four | Ordered transformation list (add/edit/reorder); each applies to previous result |

### 3.4 Dress-ups

Selection: edges/faces/whole feature; face ⇒ all its edges; tangent chains propagate;
Add/Remove pickers (current refs highlighted purple in remove mode).

| Op | Parameters |
|---|---|
| Fillet | Radius; **UseAllEdges** (fillets everything, immune to edge renames) |
| Chamfer | Type: Equal distance / Two distances / Distance+angle; Flip direction; UseAllEdges |
| Draft | Angle (default 1.5°), Neutral plane (required), Pull direction (optional edge), Reversed; fails on tangent-connected faces (fillet-after-draft) |
| Thickness | Value, Mode: Skin (only working one), Join: Arc / Intersection, Reversed (**inward default since 1.0**), self-Intersection checkbox |

### 3.5 Profile validity rules

- Solid-making ops need **closed wires** ("Failed to validate broken face" otherwise).
- Nested closed wires = holes; multiple separate wires = multiple extrusions (must
  still merge into one solid unless multi-solid Body).
- Profile sources: one sketch · solid face(s) (`AllowMultiFace`) · SubShapeBinder
  **face picked in 3D** (tree-pick fails for Revolution/Groove/Helix) · ShapeBinder
  (Pad/Pocket only).
- A consumed sketch is "used" — select-feature dialogs skip it unless
  "Allow used features" is checked.
- 1.1 experimental `MakeInternals`: closed contours become selectable faces inside the
  sketch → master-sketch workflow (ops take individual faces of one big sketch).

---

## 4. Sketcher — editing model

### 4.1 Edit-mode lifecycle

```
ENTER: Create sketch (pick plane/face) │ double-click in tree │ Edit sketch cmd
  ├─ camera aligns to sketch plane (opt. force orthographic)
  ├─ task Dialog replaces model tree; edit-mode toolbars appear
  ├─ visibility automation (prefs): hide dependents / show ext-geo sources /
  │  show attachment support / restore camera on exit
  └─ optional Section View: clip everything in front of the sketch plane
       (persisted per-sketch: SectionView property)

DURING: every geometry/constraint action = one undoable transaction

LEAVE: Close button │ Leave sketch │ Esc (pref-gated) — doc stays modified until save
```

### 4.2 Tool organization (1.x)

- **Non-edit toolbar**: Create/Edit/Attach/Reorient/Validate/Merge/Mirror sketch.
- **Edit mode**: Leave sketch, View sketch, View section.
- **Edit tools**: Toggle grid, Toggle snap, Rendering order — each with settings flyout.
- **Sketcher geometries** / **constraints** / **tools** (fillet, trim, split, extend,
  external geometry, carbon copy, transforms, offset, symmetry) / **B-spline tools** /
  **visual** (select-DoF/redundant/conflicting, show internals, virtual space).
- History: the big split happened in **0.21**; 1.0 reorganized for clarity, unified
  icons, made viewport right-click contextual, grouped tools into multi-mode "comp"
  dropdowns, and **removed** Clone/Copy/Move/RectangularArray (replaced by new
  transform tools).

### 4.3 The 1.0 tool-handler UX

- **On-View-Parameters (OVP)**: floating inputs at the cursor — Pos-OVP (x/y) and
  Dim-OVP (length/angle/radius). Typing a value **locks it and auto-creates the
  matching constraint**. Tab cycles fields. Pref: None / Dimensions only / Both.
- **Tool parameters section** in the task dialog mirrors OVP + mode dropdown + option
  checkboxes with their hotkeys.
- Keys: **M** cycles tool modes · **U/J** first option/increment · **R/F** second.
- **Continue mode**: tools re-arm after each element (separate prefs for geometry vs
  constraints); exit = right-click / Esc / other tool.
- **Auto-constraints at cursor**: suggested Coincident / PointOnObject / Horizontal /
  Vertical / Tangent (+ Symmetric on line midpoints, 1.0) shown as icons at the
  cursor, applied on click.

### 4.4 Geometry tools (interaction models)

| Tool | Modes / notes |
|---|---|
| Point / Line | Line modes (1.0): point-length-angle · point-w-h · 2 points |
| Polyline | M cycles 6 modes: ⊥-prev, tangent-prev, tangent arc, ⊥ arc L/R, unconstrained; click start point to close |
| Arc / Circle | Center mode + 3-rim-points mode (one tool, 1.0) |
| Ellipse / conic arcs | Center or axis-endpoints; parabola/hyperbola arcs; all auto-create **internal geometry** (axes, foci) as construction |
| B-spline | M: by control points / by knots (interpolation); R = periodic; U/J degree; F deletes last point |
| Rectangle | 4 modes: corner-w-h · center-w-h · 3 corners · center-2-corners; U = rounded corners, J = frame/offset |
| Regular polygon | One tool, N adjustable (U/J); adds a structural circumscribed construction circle |
| Slot / Arc slot | Slot: 2 centers + radius, angular snap; Arc slot (1.0): arc-ends or flat-ends modes |
| Fillet/Chamfer | One tool, M toggles fillet↔chamfer (1.0); U = preserve corner; pick corner point or two edges |
| Trim | Hover shows cut points; **1.0: hold-drag continuous trim**; transfers constraints, adds point-on-object at new ends |
| Extend / Split / Join | Extend lines/arcs to point/edge; Split transfers most constraints (drops angle/symmetric/block); Join merges edges into one B-spline |

### 4.5 Construction / external geometry

- **Construction** (G,N): with selection converts, without switches creation mode
  (tool icons recolor). Blue **dashed** (1.0); excluded from the 3D shape (exception:
  Revolution axis); invisible outside edit mode.
- **Internal geometry** (ellipse axes/foci, B-spline control polygon): auto-created
  construction; Show/hide internal geometry recreates/cleans unconstrained sets.
- **External geometry**: projects outside edges/vertices onto the sketch plane as
  parametric references (magenta). Scope: same coordinate system (same Body/Part or
  both global), no cycles. GeoIds are negative (§5.6).
  - ≤1.0: one tool; imports as non-defining reference that must be traced over.
  - **1.1: two tools — Projection (G,X) and Intersection (G,I)** (reference geometry
    crossed with the sketch plane); whole **faces** selectable; imported geometry is
    **defining (real) by default**, per-edge toggleable to construction; usable
    directly by offset/transform tools → explicit master-sketch workflow.
- **Carbon copy** (G,W): copies another sketch's geometry **and constraints**;
  dimensions stay expression-linked to the source; plane-parallel requirement
  (Ctrl/Ctrl+Alt overrides).

### 4.6 Selection / editing aids

- Drag: solver-driven; "improve solving while dragging" anti-flip; **1.1: group drag**
  of a multi-selection.
- Box select L→R contained / R→L crossing; double-click edge selects connected chain
  (1.0); copy/cut/paste across sketches with constraints (1.0).
- **Elements panel**: row per geometry with up to 4 hover buttons (edge/start/end/
  center); type filters (Normal/Construction/Internal/External + per-shape); extended
  info shows `TYPE(EdgeN#ID<GeoId>#VLx)`; context menu → constraints, toggle
  construction, layer, delete.
- **Layers**: 3 hardcoded visual layers — 0 solid, 1 dashed, 2 hidden.
- **Snap bar**: snap to grid (20% of spacing) · snap to objects (edges, midpoints) ·
  snap angle (Ctrl, 5° steps). Creation aid only, adds no constraints.
- **Grid**: auto-spacing rescales with zoom; per-sketch ShowGrid/GridSize props.
- **Rendering order**: drag-to-reorder normal/construction/external draw+pick priority.

### 4.7 Sketch-level transform tools (all 1.0 unless noted)

| Tool | Behavior |
|---|---|
| Translate/Array (W) | Move (vector) or Copy with n copies + rows → rectangular arrays; "apply equal constraints" = clone-like |
| Rotate/Polar (Z,P) | Center + angle; copies via U/J (0 = rotate originals) |
| Scale (Z,P,S) | Base point + ratio picks or typed factor; U keeps originals; external constraints dropped |
| Offset (Z,T) | Arc vs intersection corner styles (M); U deletes originals; J adds offset dimension; open profiles close both sides; no ellipses/B-splines (external geometry supported from 1.1) |
| Symmetry (Z,S) | Reworked: select geometry → invoke → pick mirror line/axis/point; U mirror-in-place, J add symmetric constraints |
| Remove axes alignment (Z,R) | Swaps H/V for parallel/perpendicular so a group can rotate rigidly |
| Mirror / Merge sketch | Non-edit-mode: new mirrored sketch(es) about X/Y/origin; merge sketches into one |

### 4.8 Validation / repair (non-edit mode)

- Missing coincidences (tolerance, main DXF fix) · Invalid constraints (empty links —
  typical toponaming fallout) · Degenerated geometry (zero-length/radius) · Reversed
  external geometry · Constraint orientation locking (tangent/perpendicular flip
  state).
- In-edit diagnostics = solver messages (§5.4) + select-redundant/conflicting/DoF
  visual tools.

---

## 5. Sketcher — constraint system

### 5.1 Geometric constraints

| Constraint | Key | Valid selections → effect | Glyph |
|---|---|---|---|
| Coincident (unified, 1.0) | C | points → merge; point+edge → on-object; 2 circular edges → concentric | merged dot (red when constrained) |
| Point on object | O | point(s) + edge(s); lines infinite, open curves virtually extended | small icon at point |
| Horizontal/Vertical auto (1.0) | A | lines or point pairs → whichever is closer | — / \| bar at midpoint |
| Horizontal / Vertical | H / V | line(s); 2 points | bar icon |
| Parallel | P | 2+ lines | paired slanted icons, indexed |
| Perpendicular | N | 2 edges (one straight); 2 endpoints (→ +coincident); endpoint+edge; point+2 edges (`PerpendicularViaPoint`, helpers auto-added) | ⊥ at both edges |
| Tangent / Collinear | T | 2 edges; 2 endpoints (smooth/sharp joint — replaces existing coincident: constraint substitution); endpoint+edge; point+2 edges (`TangentViaPoint`); 2 lines → collinear | tangent icons |
| Equal | E | 2+ same-type edges (circle↔arc OK, line+circle rejected); ellipses match both radii | "=" per edge, group-indexed |
| Symmetric | S | 2 points + line/axis; 2 points + point; line + point | >\|< arrows |
| Block | K,B | edge(s) frozen with one constraint (B-spline workhorse) | lock icon |
| Snell's law | K,W | 2 line endpoints + interface edge; value n2/n1 | datum label at interface |

### 5.2 Dimensional constraints

| Constraint | Key | Selections | Notes |
|---|---|---|---|
| **Dimension** (context, 1.0) | D | anything (not B-spline edges) | one tool → distance/X/Y/radius/angle by selection + click position; M cycles; pref: single tool / separated / both |
| Distance | K,D | line length · 2 points · point-line ⊥ · curve-curve gap (0.21) · **arc length (1.0)** | red driving / blue reference |
| DistanceX / DistanceY | L / I | 2 points, 1 point (from origin), line | axis-aligned dimension |
| Radiam (auto R/⌀, 0.20) | K,S | circular edges: radius for arcs, diameter for circles; multi-select → first dimensioned + rest Equal | |
| Radius / Diameter | K,R / K,O | circles/arcs (radius also B-spline weight circles) | R / ⌀ label + leader |
| Angle | K,A | line vs X axis · 2 lines · arc aperture · 2 edges + point (`AngleViaPoint`) | arc dimension, full extension lines (1.0) |
| Lock | K,L | point(s) → DistanceX+Y vs origin (or vs last point) | H+V pair |

### 5.3 Driving / reference / active states

| State | Color | Meaning |
|---|---|---|
| Driving | red `#FF2600` | Constrains; value dialog on creation (pref) |
| Reference (driven) | blue `#0026FF` | Measured output; no value assignment; usable in expressions |
| Expression-bound | orange `#FF7F26` | Value locked, driven by expression |
| Deactivated (K,Z) | grey `#7F7F7F` | Kept + valued but disabled |

Toggle: K,X (driving↔reference; also flips creation mode of dimensional tools).

### 5.4 Solver (planegcs)

```
SketchObject (doc object, Constraints property)
   └─ Sketcher::Sketch — maps Constraint list → GCS constraints on double* params
        └─ GCS::System (planegcs)
             ├─ algorithms: DogLeg (default) · LevenbergMarquardt · BFGS  (+SQP sub-solves)
             ├─ diagnosis: QR decomposition of Jacobian → DoF = params − rank
             │    rank deficiency → redundant / conflicting / partially-redundant / malformed
             └─ drag: initMove() snapshot → temp point-to-cursor constraint → minimize
```

- Parameter counts: point 2, line 4, circle 3, arc 5, …
- "Advanced solver control" (pref-gated task section): algorithm, max iterations,
  convergence, QR (Eigen dense/sparse), redundant-solver params, debug, manual Solve.
- **Solver messages** (top of task dialog, first applicable; clickable indices select
  offenders): Empty sketch · **Fully constrained** (geometry → green) ·
  Under-constrained: n DoF · Over-constrained (conflicting) · Redundant ·
  Partially redundant · Malformed · Solver failed to converge.
- Conflicting/redundant get **no distinct 3D color** — feedback is the message +
  select tools (select unconstrained DoF Z,F / redundant Z,P,R / conflicting Z,P,C).

Edit-mode default colors (source: `EditModeCoinManagerParameters.cpp`):

| Item | Color |
|---|---|
| Normal geometry | white `#FFFFFF` |
| Fully constrained sketch | green `#00FF00` (single element 1.0: `#80D0A0`) |
| Construction | blue `#0000DC` (fully constrained `#8FA9FD`) |
| Internal alignment | `#B2B27F` |
| External geometry | magenta `#CC3399` |
| Invalid sketch | orange `#FF6D00` |
| Constraint symbols / driving | red `#FF2600` |
| Points | white normal · blue construction/center · red coincident (1.0) |

### 5.5 Creating & editing constraints

- Modes: continue mode (invoke → click repeatedly) or pre-select → invoke; viewport
  right-click menu (1.0).
- **Auto remove redundants** (pref, default off): new constraint deletes the older one
  it made redundant.
- Double-click a dimension → value dialog with **fx expression icon**; expressions
  reference `Constraints.Name` (same sketch), `<<SketchLabel>>.Constraints.Name`,
  `Spreadsheet.Alias`, `VarSet.Prop` (1.0; conditionals allowed). All unit-aware.
- **Naming** (F2 / dialog field) needed for stable expression refs; stricter charset
  than labels.
- Labels draggable; stored per constraint (`LabelDistance`, `LabelPosition`).
- **Constraints panel**: filters (All/Geometric/Datums/Named/Reference/Selected/
  Associated), per-row hide checkbox (glyph only — constraint stays active), hover
  preselects in 3D, context menu (change value, toggle reference, deactivate, rename,
  select elements).
- **Virtual space** (Z,Z): park constraint glyphs in a second space to declutter
  (`InVirtualSpace` flag).

### 5.6 Internal representation (relevant to printCAD's model)

- Storage: `PropertyConstraintList Constraints` on `Sketcher::SketchObject`; outputs:
  `FullyConstrained`, `DoF`, `ConflictingConstraints`, `RedundantConstraints`, ….
- `Sketcher.Constraint` fields: `Type, First, FirstPos, Second, SecondPos, Third,
  ThirdPos, Value, Name, Driving, InVirtualSpace, IsActive, LabelDistance,
  LabelPosition` — i.e. **one record type for all constraints, up to 3 (geo,pos)
  references + 1 scalar**.
- **GeoId**: sketch geometry 0-based (GUI shows 1-based); `-1` = X axis, `-2` = Y
  axis, `-n (n≥3)` = external geometry (ExternalGeometry[0] → −3, …).
- **PosId**: 0 = whole edge, 1 = start, 2 = end, 3 = mid/center, n = n-th B-spline pole.
- API: `addConstraint`, `delConstraint(s)`, `setDatum`, `set/getDriving`,
  `set/getActive`, `renameConstraint`, `solve() → 0 on success`.

### 5.7 Constraint-system deltas 1.0 → 1.1

- 1.0: contextual Dimension tool, unified H/V and Coincident tools, arc-length
  distance, direct B-spline tangency, midpoint symmetric auto-constraint, icon/color
  restyle, OVP dimensioning, constraint copy/paste.
- 1.1: **no new constraint types** — focus was external geometry (defining
  projection/intersection). Third-party reviews claim a constraint-search box and
  free label repositioning; not in official notes — *unverified*.

---

## 6. Recent development & direction of travel (for printCAD)

### 6.1 Post-1.1 activity (mid-2026)

- CalVer cadence: 26.3 branches Sep 2026; then 27.1 (quality-focused) / 27.2 / 27.3.
  Weekly dev builds + "WIP Wednesday" posts.
- Landing in main: Sketcher constraint-label placement, PD sketch-creation workflow
  prefs, suppress-recompute fixes, Assembly snapshots, Hole cosmetic threads,
  TechDraw expression editor.
- Funded work (FPA grants): Sketcher refactoring + **offset/restricted curve types**
  (AjinkyaDahale), quarterly Sketcher/Assembly/PD maintenance (PaddleStroke),
  **Assembly solver abstraction** → pluggable MuJoCo/Chrono backends, rendering +
  selection-pipeline modernization, **multithreaded recompute architecture** (in
  progress, did not ship in 1.1), parametric **thread feature** on cylindrical faces.
- Ondsel shut down Nov 2024; everything (Assembly WB, ondsel-solver, VarSets,
  Sketcher UX) was upstreamed first. FPA now funds maintenance via grants + bug
  bounties.

### 6.2 Architectural lessons

| Lesson | Detail |
|---|---|
| **Toponaming is THE lesson** | 3 years retrofitting element maps onto OCCT, still called a "mitigation", still per-workbench in 2026. Build stable topological identity (persistent element naming through feature history) into the kernel layer from day one |
| Master-sketch is the direction | External geometry as first-class *defining* geometry with projection/intersection modes; `MakeInternals` faces; one sketch driving many features |
| In-viewport editing is baseline | On-screen dimension input while drawing, input hints, snapping, transparent feature previews, gizmo draggers on every feature |
| Uniform constraint record | One record: type + ≤3 (GeoId,PosId) refs + scalar + flags (driving/active/virtual-space) covers the entire constraint system |
| Dialog ⇄ property equivalence | Every task-dialog control maps to a data property; both paths recompute. Keeps scripting, undo, and UI honest |
| Linear feature chain | BaseFeature links + AddSubShape tool solids make the Body a simple, repairable linked list — reorder/delete/suppress are link surgery |
| Attachment as an engine | One attacher (mode + refs + local offset) shared by sketches, datums, primitives — not per-type ad hoc placement |
| Known pain points they're fixing (don't replicate) | Single-threaded recompute, aging Coin3D rendering/selection pipeline, OCCT boolean fragility (helices, thickness), suppression dependency-cascade bugs (#19308/#20942) |
| Process | CalVer + fixed cadence + paid maintenance grants: sustained incremental delivery beat rewrites |

---

## 7. Sources

Research compiled July 2026 from:

- FreeCAD wiki (via the official `FreeCAD/FreeCAD-documentation` GitHub mirror):
  workbench pages, every PartDesign/Sketcher tool page, `Feature_editing`,
  `Topological_naming_problem`, `Part_EditAttachment`, `Sketcher_Dialog`,
  `Expressions`, `Release_notes_0.21 / 1.0 / 1.1`.
- FreeCAD source (`FreeCAD/FreeCAD` main): `src/Mod/PartDesign/App/{Feature,
  FeatureAddSub, FeatureDressUp, FeatureExtrude, Body}.h/.cpp`,
  `src/Mod/Sketcher/App/SketchObject.h`, `Constraint.pyi`,
  `src/Mod/Sketcher/Gui/EditModeCoinManagerParameters.cpp` (colors),
  `TaskSketcherConstraints.*`.
- Blog: 1.0 / 1.0.x / 1.1 / 1.1.1 release posts, versioning-scheme post (FEP-0003),
  WIP Wednesdays (Apr–Jul 2026), FPA grant announcements (2025 year-in-review,
  Q2 2026), GSoC 2026, Ondsel Onwards Fund; ondsel.com goodbye post.
- GitHub PRs/issues: #17736/#18697/#17615 (external geometry), #18273 (group drag),
  #21794 (two-sided extrude), #22389 (spacing patterns), #15744 (Whitworth/taper
  threads), #19052/#19167 (Hole panel), #12096/#12412 (Suppressed), #11392/#14433
  (UpToShape), #8716 (layers), #19308/#20942 (suppression semantics),
  FreeCAD-Enhancement-Proposals FEP-0003.
- realthunder's Topological-Naming-Algorithm wiki (assembly3), planegcs README,
  third-party 1.1 reviews (Phoronix, 9to5linux, freecadextreme — cross-checked;
  unverified claims flagged inline).
