# printCAD Initial Plan

## Vision

- Linux Wayland-native CAD focused on designing parametric parts for FDM/SLA 3D printing.
- Built entirely in Rust with a Vulkan renderer.
- Modular workbenches (initially Sketch + Part Design) with clean abstractions for future add-ons (macros, alternative kernels, renderers).

## High-Level Requirements

- **Platform**: Wayland first-class support, strong input handling, optional XWayland fallback later.
- **Rendering**: Vulkan backend with hooks for replacing the renderer without touching higher layers.
- **Parametric Core**: Feature tree + dependency graph, transactions, undo/redo, constraint solving.
- **Extensibility**: Workbenches, macro scripting, and kernel/render backends loadable via traits or dynamic plugins.
- **Persistence**: Native `.prtcad` (printCAD) container plus first-class STEP import/export for interoperability.

## Architecture Overview

- **App shell**: `winit` on its Wayland backend, driving a render-on-demand loop rather than an async one — frames are produced only while something is moving, pending or animating.
- **UI layer**: `egui` through `egui-winit`, drawn by the project's own Vulkan backend. The panels sit above a design system (`ui_kit`) so a workbench never reaches for a colour or a size itself.
- **Core services**:
  - Document manager with versioned history.
  - Feature/constraint graph engine with dependency tracking.
  - Geometry kernel abstraction (`Kernel` trait) implemented with the pure-Rust ogeom kernel.
  - Rendering service exposing a `RenderBackend` trait (default Vulkan).
  - Persistence service (project serialization in JSON + binary payloads).
- **Workbench system**: Each workbench implements a `Workbench` trait that registers tools, commands, property panes, and feature nodes.
- **Plugin hooks**: Command registry and document API exposed so future macro/scripting engines can automate operations.

## Technology Choices

- **Geometry kernel**: [ogeom](https://github.com/gilbertorconde/ogeom-rs), a pure-Rust B-rep kernel (booleans, meshing, STEP IO), wrapped through a dedicated `kernel_ogeom` crate. The kernel stays behind traits so alternatives can be slotted in later.
- **Math layer**: `glam` throughout, with an `axes` crate on top so nothing hardcodes which way is up — the axis preset decides.
- **Constraint solving**: a Levenberg–Marquardt solver written for the sketcher, over a uniform constraint record that also carries its own diagnostics.
- **Rendering**: Vulkan through `ash`, behind a `RenderBackend` trait that takes data and returns pixels.
- **Wayland integration**: `winit`, which also gives X11 for free.

## Parametric & Data Model

- Directed acyclic graph capturing sketches, reference geometry, and feature parameters.
- Transaction-based edits enabling undo/redo.
- Constraint solver pipeline: parameter changes → sketch solve → kernel rebuild → mesh/tessellation update.
- Dirty-flag propagation to limit recomputes and keep interaction responsive.

## File Format Strategy

- **Native format**: `.prtcad`, a printCAD-exclusive package that stores the document graph, feature tree, workbench state, macro bindings, and cached tessellations. Implementation detail: tar archive (optionally compressed with gzip or zstd) bundling JSON metadata and binary blobs.
- **STEP interoperability**: kernel-powered STEP import/export remains primary for exchanging models. Optionally emit/refresh a `.step` snapshot on every project save.
- **Round-trip behavior**: Loading `.prtcad` restores full parametric fidelity; importing `.step` creates base bodies without historical features, mirroring other CAD workflows.

## Workbench MVPs

- **Sketch Workbench**
  - 2D drawing primitives, dimensional/geometric constraints, reference planes.
  - Solver results produce profiles consumable by Part Design.
  - Visualization overlays for constraints and degrees of freedom.
- **Part Design Workbench**
  - Feature stack: pad, pocket, revolve, fillet, chamfer.
  - Feature tree editor with parameter forms.
  - Uses the kernel for B-Rep ops and tessellation for viewport display/export (STL/STEP).

## Rendering & Interaction

- Scene graph for tessellated solids plus sketch overlays.
- Camera controller (orbit/pan/zoom), section planes, visual styles (wireframe, shaded, shaded + edges).
- GPU picking using ID buffers, gizmos for constraints and feature handles.
- Render backend trait so alternative renderers can be introduced without touching higher layers.

## Modularity & Extensibility

- Workbench registry managing tool activation and UI docking.
- Plugin loader (`libloading` or feature-gated crates) for future modules.
- Macro infrastructure reserved via stable command/document APIs; future scripting engine (Rhai/Python) can bind into these.
- Clear separation between kernel, render backend, UI, and workbench logic to encourage experimentation.

## Roadmap & Needed Work

1. **Foundation**
   - Scaffold workspace with separate crates (`app_shell`, `core_document`, `render_vk`, `kernel_api`, `kernel_ogeom`, `wb_sketch`, `wb_part`).
   - Bring up Wayland window + Vulkan swapchain, event loop integration, logging/telemetry.
   - Define core traits (workbench, kernel, render backend, document services) and establish serialization stubs.
2. **Sketch MVP**
   - Implement sketch document structures, constraint graph, and solver.
   - Build sketch UI tools (line/arc/circle, constraints palette) and viewport overlays.
   - Ensure param changes propagate to the document and mark dependent features dirty.
3. **Part Design MVP**
   - Integrate the kernel; implement pad/pocket/revolve operations.
   - Create feature tree UI, parameter editors, and regen pipeline.
   - Generate triangulated meshes for viewport and STL export.
4. **Parametric Engine**
   - Finalize dependency graph, recompute scheduler, and transactional undo/redo.
   - Introduce configuration management for multi-body workflows.
   - Add persistence (project save/load) with versioning.
5. **Refinement**
   - Implement fillet/chamfer, shell, pattern features.
   - Improve selection/picking, add measurement tools, section views, and visual styles.
   - Harden kernel integration, optimize tessellation quality vs. performance, expand export/import formats.
6. **Modularity Enhancements**
   - Dynamic workbench loading, feature toggles, and plugin discovery.
   - Renderer abstraction finalized and alternative backend proof-of-concept.
   - Macro API surface defined with command registry exposure and initial scripting hooks.

## Risks & Open Questions

- Kernel co-evolution (ogeom lives in its own repo); pin revisions and bump deliberately.
- Constraint solver performance for complex sketches—prototype early.
- egui is an immediate-mode toolkit: docking and free-floating panels are the project's to build, not the toolkit's to provide.
- Future cross-platform requirements might necessitate different windowing/input stacks; keep layers clean.

## Where this stands

The foundation, both workbenches, persistence and the document server are
built; `PROJECT_STEPS.md` tracks what landed, and its last section is the
current list of what has not. The nearest items are STEP export, assembly
constraints, and the hand-off to slicing that the whole thing is for.
