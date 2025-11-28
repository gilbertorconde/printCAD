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

- ✅ Rewrote the camera controller based on `camera_movements.md`.
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
  `CommandDescriptor`, etc.).

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

## 10. Outstanding / Next Steps (for future LLMs)

1. **Tree interaction & modes** (see `docs/WB_IMP.md`):
   - Double-clicking tree items to set active document object and enter the appropriate editing mode.
   - Restoring previous camera pose when exiting sketch mode.
2. **Plane selection dialog** for “Create Sketch” (base planes or selected faces).
3. **Sketch tool improvements**:
   - Constraint solving, dimension inputs, snapping/grid overlays.
   - Persisting tool state in workbench storage.
4. **Body/workbench flows**:
   - Enforce active body selection before sketch/part tools.
   - Mirror the same flows for PartDesign (pads/pockets/etc.).
5. **Document tree UI**:
   - Real tree view with bodies/features/sketches; double-click events.
6. **Rendering**:
   - Investigate why sketch geometry is not yet visible (validate tessellation submission).
7. **Camera**:
   - Store/restore camera pose when switching in/out of editing modes.
8. **Persistence**:
   - Hook document save/load to the TAR-based `.prtcad` format.
9. **Workbench plugins**:
   - Expose APIs for third-party workbenches (UI hooks, debug panel integration, etc.).

Keep this file updated whenever new milestones are reached so every LLM agent can quickly align with
the project history and roadmap.
