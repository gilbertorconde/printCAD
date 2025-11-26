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

- **App shell**: Wayland windowing via `winit` (Wayland backend) or `smithay-client-toolkit` integrated with a central async event loop.
- **UI layer**: Prefer `egui + egui_winit_vulkano` for rapid tooling; keep UI behind an adapter trait to allow swapping with `iced` or other frontends.
- **Core services**:
  - Document manager with versioned history.
  - Feature/constraint graph engine with dependency tracking.
  - Geometry kernel abstraction (`Kernel` trait) implemented initially with OCCT bindings.
  - Rendering service exposing a `RenderBackend` trait (default Vulkan).
  - Persistence service (project serialization in JSON + binary payloads).
- **Workbench system**: Each workbench implements a `Workbench` trait that registers tools, commands, property panes, and feature nodes.
- **Plugin hooks**: Command registry and document API exposed so future macro/scripting engines can automate operations.

## Technology Choices

- **Geometry kernel**: Start with OCCT for robust B-Rep, booleans, meshing, and STEP/IGES IO. Wrap through a dedicated `kernel_occt` crate. Keep the kernel behind traits so CGAL or custom kernels can be slotted in later.
- **Math layer**: Use `nalgebra`/`glam` for light linear algebra; consider GLM-style APIs via `glam` if ergonomic needs arise. Eigen is unnecessary unless a C++ dependency mandates it.
- **Constraint solving**: Lightweight solver built in Rust (e.g., `ncollide` + custom) for 2D sketches, with the option to integrate CGAL constraint solvers if needed.
- **Rendering**: Vulkan with `vulkano` (higher-level, safer) or `ash` (lower-level control). Keep renderer modular for future Metal/OpenGL/OpenXR targets.
- **UI toolkit**: Begin with `egui` for immediate-mode editing tools; evaluate `iced` once docking/layout needs increase.
- **Wayland integration**: `winit` provides Wayland support and input abstractions; only drop to `smithay-client-toolkit` if tighter control is required. SDL3 is unnecessary unless cross-platform goals expand.

## Parametric & Data Model

- Directed acyclic graph capturing sketches, reference geometry, and feature parameters.
- Transaction-based edits enabling undo/redo.
- Constraint solver pipeline: parameter changes → sketch solve → kernel rebuild → mesh/tessellation update.
- Dirty-flag propagation to limit recomputes and keep interaction responsive.

## File Format Strategy

- **Native format**: `.prtcad`, a printCAD-exclusive package that stores the document graph, feature tree, workbench state, macro bindings, and cached tessellations. Implementation detail: compressed archive (ZIP/zstd) bundling JSON metadata and binary blobs.
- **STEP interoperability**: OCCT-powered STEP import/export remains primary for exchanging models. Optionally emit/refresh a `.step` snapshot on every project save.
- **Round-trip behavior**: Loading `.prtcad` restores full parametric fidelity; importing `.step` creates base bodies without historical features, mirroring other CAD workflows.

## Workbench MVPs

- **Sketch Workbench**
  - 2D drawing primitives, dimensional/geometric constraints, reference planes.
  - Solver results produce profiles consumable by Part Design.
  - Visualization overlays for constraints and degrees of freedom.
- **Part Design Workbench**
  - Feature stack: pad, pocket, revolve, fillet, chamfer.
  - Feature tree editor with parameter forms.
  - Uses OCCT for B-Rep ops and tessellation for viewport display/export (STL/STEP).

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
   - Scaffold workspace with separate crates (`app_shell`, `core_document`, `render_vk`, `kernel_api`, `kernel_occt`, `wb_sketch`, `wb_part`).
   - Bring up Wayland window + Vulkan swapchain, event loop integration, logging/telemetry.
   - Define core traits (workbench, kernel, render backend, document services) and establish serialization stubs.
2. **Sketch MVP**
   - Implement sketch document structures, constraint graph, and solver.
   - Build sketch UI tools (line/arc/circle, constraints palette) and viewport overlays.
   - Ensure param changes propagate to the document and mark dependent features dirty.
3. **Part Design MVP**
   - Integrate OCCT bindings; implement pad/pocket/revolve operations.
   - Create feature tree UI, parameter editors, and regen pipeline.
   - Generate triangulated meshes for viewport and STL export.
4. **Parametric Engine**
   - Finalize dependency graph, recompute scheduler, and transactional undo/redo.
   - Introduce configuration management for multi-body workflows.
   - Add persistence (project save/load) with versioning.
5. **Refinement**
   - Implement fillet/chamfer, shell, pattern features.
   - Improve selection/picking, add measurement tools, section views, and visual styles.
   - Harden OCCT integration, optimize tessellation quality vs. performance, expand export/import formats.
6. **Modularity Enhancements**
   - Dynamic workbench loading, feature toggles, and plugin discovery.
   - Renderer abstraction finalized and alternative backend proof-of-concept.
   - Macro API surface defined with command registry exposure and initial scripting hooks.

## Risks & Open Questions

- OCCT binding maintenance and licensing considerations; need build automation.
- Constraint solver performance for complex sketches—prototype early.
- UI toolkit commitment (egui vs iced) affects docking and layout flexibility.
- Future cross-platform requirements might necessitate different windowing/input stacks; keep layers clean.

## Immediate Next Steps

1. Prototype OCCT binding and Vulkan + egui integration to derisk core tech choices.
2. Lock crate layout and coding standards.
3. Begin implementing Foundation milestone per roadmap.
