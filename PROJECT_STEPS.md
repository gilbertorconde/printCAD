# printCAD Development Steps (LLM Reference)

This document summarizes the high-level steps already implemented in the project so future LLM
assistants have a quick reference. Each section links back to the relevant topics described in
`docs/WB_IMP.md` and `docs/plan.md`.

---

## 1. Foundation

- ✅ Scaffolded the multi-crate workspace (`app_shell`, `core_document`, `render_vk`,
  `wb_sketch`, `wb_part`, etc.).
- ✅ Brought up the Wayland window + Vulkan renderer with egui overlay.
- ✅ Added logging infrastructure and settings persistence (FPS cap, MSAA, etc.).

## 2. Camera & View Controls

- ✅ Rewrote the camera controller on the focal-distance model in `camera_system.md`.
- ✅ Added axis presets (`horizontal`, `vertical`, `depth`) and exposed them in settings.
- ✅ Fixed orbit/pan/zoom parity issues for all axis presets.
- ✅ Implemented the orientation cube: chamfered inner cube, labeled faces via SVG, interactive
  edges/corners/arrows, parity fixes, proper UVs, and clickable rotations.

## 3. Rendering Pipeline Enhancements

- ✅ Implemented MSAA, depth buffer, and ensured UI rendering survived the change.
- ✅ Modularized `render_vk` (core/picking/mesh/surface/util modules).
- ✅ Added GPU picking (ID + depth buffer) plus hover coordinate display on the status bar.
- ✅ Added selection manager + highlight feedback.

## 4. Logging & Diagnostics

- ✅ Introduced an in-app log panel (toggle in settings) with info/warn/error levels.
- ✅ Forwarded workbench logs to the panel and to tracing.

## 5. Axis Abstraction

- ✅ Created the `axes` crate and replaced `.x/.y/.z` usages with axis-aware helpers across camera,
  picking, orientation cube, render math, and settings.
- ✅ Added axis presets to settings (with proper control-handness handling).

## 6. Workbench API & Modularity

- ✅ Defined `Workbench` trait hooks: lifecycle, input handling, UI panels, settings, `finish_editing`.
- ✅ Added `WorkbenchRuntimeContext` with document access, camera info, picking state, logging, etc.
- ✅ Refactored the app shell to defer workbench activation/deactivation/input until after rendering
  mutable borrows were released.
- ✅ Added `WorkbenchFeature` trait + generic feature tree storing type-erased JSON payloads.
- ✅ Documented the API in `docs/WORKBENCH_GUIDE.md` and `docs/WB_IMP.md`.

## 7. Document Model & Persistence

- ✅ Redesigned the document model to store generic feature nodes (`FeatureTree`) plus per-workbench
  storage and asset references.
- ✅ Changed `.prtcad` package format to a TAR-based archive with `document.json` + `assets/`.
- ✅ Introduced document runtime context (`DocumentService`, `WorkbenchContext`, `ToolDescriptor`,
  `WorkbenchDescriptor`, etc.).

## 8. Sketch Workbench MVP

- ✅ Added `SketchFeature` + serialization.
- ✅ Implemented viewport rendering for sketch geometry (points/lines/circles/arcs) via tessellation.
- ✅ Added tools (Line, Arc, Circle) with multi-click interactions (2-click line/circle, 3-click arc).
- ✅ Implemented “Create Sketch” action: creates sketch feature, adds it to the tree, selects it, and
  orients the camera to the sketch plane.
- ✅ Introduced sketch editing mode vs. document selection (active document object vs. editing mode).
- ✅ Added “Exit Sketch Mode” button in the left panel (sketch remains selected in the tree).
- ✅ Moved tool buttons to the top bar, differentiating action vs. radio tools (Create Sketch + sketch
  tools).
- ✅ Disabled sketch tools until a sketch exists; removed auto-creation on workbench activation.

## 9. Picking & Selection Integration

- ✅ Hover coordinates displayed in the bottom bar, using axis labels (per preset).
- ✅ Added hovered/selected body IDs in runtime context and UI status.
- ✅ Ensured orientation cube/pivot UI uses picking to rotate around hovered objects.

## 10. Document Server & Persistence

- ✅ Moved file ownership behind a `DocumentServer` trait: `printcad-serverd`, one daemon per
  document over a UNIX socket, with direct file I/O as the fallback.
- ✅ Replaced snapshot undo with per-edit inverse operations recorded by the document's mutators,
  and an op log the daemon stores beside the file.
- ✅ Packed and unpacked `.prtcad` archives off the UI thread, with the container bytes crossing
  the wire beside the message rather than encoded into it.

## 11. Sketcher & Part Design

- ✅ Constraint solver (Levenberg–Marquardt) with degrees-of-freedom reporting and conflict
  diagnostics; the origin and its axes take constraints like any geometry.
- ✅ Sketch tools: line, arc, circle, rectangle, spline, with snapping, dragging, box select and
  typed dimensions.
- ✅ Part Design features: pad, pocket, revolution, groove, loft, pipe, helix, primitives, hole,
  fillet, chamfer, draft, thickness, patterns, booleans — each editable in a task panel and
  rebuilt through the dependency graph.
- ✅ Sketch-on-face with re-resolved face references, and a planar view lock while editing.

## 12. Shell & Interaction

- ✅ Design system (`ui_kit`): tokens, widgets, vendored icons and fonts, used by the panels and
  the workbenches.
- ✅ Start page, command palette (Ctrl+K), Preferences with a page per group, log panel.
- ✅ Render on demand: frames only while something is moving, pending or animating.
- ✅ 6-DoF mouse support through the external `sixdof` crate, with a Preferences page that assigns
  each movement of the puck.

## 13. Outstanding / Next Steps

1. **STEP export** — the kernel writes STEP; the app has no command for it yet.
2. **Assembly constraints** — placement between bodies, not just imported transforms.
3. **Slicing hand-off** — the point of the whole thing: get a part to a printer.
4. **Kernel gaps** — tests marked `#[ignore = "kernel: …"]` document what the geometry kernel
   cannot do yet; each names the issue that tracks it.
5. **Face and edge identity** — faces are matched geometrically rather than by kernel id, which is
   what limits dress-up selection and up-to-face terminations.

Keep this file updated whenever new milestones are reached so every agent can quickly align with
the project history and roadmap.
