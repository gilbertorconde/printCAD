# printCAD — agent notes

Linux-native parametric CAD app aimed at FDM/SLA printing.
Rust workspace + Vulkan (ash) + egui + OpenCASCADE via a hand-written C++ shim.

**Never reference FreeCAD by name** anywhere in the project — no code,
comments, identifiers, file or folder names, docs, UI strings, or commit
messages. Describe conventions on their own terms, not by attribution.
(This line is the single sanctioned mention.)

## Commands

```bash
cargo run -p app_shell            # launch the app (needs Vulkan + Wayland/X11)
cargo test --workspace            # full suite (~200 tests)
cargo clippy --workspace --all-targets   # CI enforces -D warnings
cargo fmt --all                   # CI enforces --check
```

- Requires OpenCASCADE dev headers (`opencascade` on Arch, `libocct-*-dev` on
  Debian). `kernel_occt/build.rs` auto-detects TKDESTEP (≥7.8) vs legacy TKSTEP.
- STEP tests use the bundled fixture `crates/kernel_occt/tests/data/box.step`;
  set `PRINTCAD_TEST_STEP_FILE` to test against a richer model.
- Vulkan validation layers, when installed, are routed into `tracing`
  (target `printcad.vulkan`). Keep the app validation-clean.

## Crate map / dataflow

- `kernel_api` — pure data contract (TriMesh, ProfileWire, SolidOp/SweepKind,
  TessellationSettings). No geometry code.
- `kernel_occt` — OCCT FFI (`cpp/step_loader.cpp`, one TU). STEP import +
  `execute_solid_chain` (extrude/revolve + fuse/cut). All errors cross the FFI
  as strings; never unwind across it.
- `core_document` — Document (feature tree DAG, bodies, tar `.prtcad`
  persistence), `Workbench` trait + runtime context, snapshot undo
  (`undo.rs`), workbench registry (`service.rs`).
- `workbenches/wb_sketch` — sketcher: `tools.rs` (state machine), `snap.rs`,
  `solver.rs` (Gauss-Newton/LM, 17 constraint types), `profile.rs` (closed-wire
  extraction), `overlay.rs` (screen-space rendering while editing).
- `workbenches/wb_part` — Pad/Pocket/Revolution/Groove features; `build.rs`
  translates a body's feature history into kernel `SolidOp` chains.
- `render_vk` — data-only renderer (`FrameSubmission` in, pixels out). GPU
  picking with async readback; per-body mesh cache keyed by (id, revision).
- `app_shell` — binary. `app/` modules: `frame.rs` (per-frame loop),
  `input.rs` (events, selection), `commands.rs` (UI command application),
  `recompute.rs` (parametric rebuild driver), `workbench_host.rs` (ctx
  plumbing), `kernel_worker.rs` (OCCT thread).

Recompute loop: workbench edits document → features marked dirty via the
dependency DAG → `drive_part_recompute` (each frame) builds `SolidOp` chains →
kernel worker thread → results land in the document's imported-geometry
sidecar → rendered/picked like any body.

## Invariants — violate these and things break subtly

- **`app/gfx.rs` field order IS the teardown contract** (struct fields drop in
  *declaration* order): renderer before window. Do not reorder.
- **UiLayer must never own state the host mutates.** `active_tool` and
  `active_workbench` are seeded from `UiFrameInputs` every frame. A parallel
  copy in the UI caused an infinite New-Body loop once. Panel-hook ctx
  write-backs (logs, orient requests, created features) must be propagated
  through `LeftPanelResult`, never dropped.
- **Adding a UI action** = one variant in `ui/commands.rs` + one arm in
  `app/commands.rs::apply_ui_commands` (two-phase dispatch preserves ordering).
- **`FeatureNode.seq` is THE build-history ordering key.** `created_at` has
  millisecond ties that order randomly — never sort history by it.
- **Undo is a memory clone, not serde** — serde would drop the
  `#[serde(skip)]` sidecars (asset/BRep blobs, Arc'd). Every document mutation
  must go through something that calls `mark_dirty()` (bumps `mutation_seq`,
  which undo uses for change detection). Solids are derived state: undo/redo
  re-marks all part features dirty.
- **OCCT is not thread-safe across kernel instances in one process** (real
  SIGSEGV). All OCCT work goes through the single kernel-worker thread;
  kernel_occt tests serialize on the `OCCT_SERIAL` mutex — copy that pattern
  into any new test file there.
- **Sketch endpoint snapping REUSES point ids** — that shared-vertex topology
  is what makes profiles closed for `profile::extract_wires`. Don't create
  coincident duplicate points.
- **Pocket/Groove cut AGAINST the sketch normal by default** (a face
  sketch's normal points out of the material, so the default digs in).
- **NDC is Y-down**: the camera bakes the Vulkan Y flip into `view_proj`.
  Transform helpers live in `core_document::runtime` (ctx methods + free
  functions) — mirror them, never re-derive with a different convention.
- **Camera orientation is preset-relative** (`q·(−depth)=forward`,
  `q·vertical=up` in the active axis preset, default Z-up). Never build
  orientation quats against a hardcoded XYZ basis.
- Renderer hot path has **no `queue_wait_idle`/`device_wait_idle`**: picking
  uses per-in-flight staging slots resolved after the fence wait; buffer
  destruction goes through the `MeshCache` retire queue. Keep it that way.
- Serde compatibility: new fields on persisted types (features, sketch) take
  `#[serde(default)]` so old `.prtcad` files keep loading.

## Interaction model (current bindings)

MMB drag = orbit (MMB click = pivot pick) · RMB drag = pan · wheel = zoom ·
LMB = select (click sketch → tree-select; click solid → face-first, double
click → whole body; LMB drag in sketch = box select; ctrl = additive).
While editing a sketch the view is locked planar (orbit + cube rotation
disabled; pan/zoom/roll allowed).

## Testing conventions

- Sketcher end-to-end tests drive `on_input` with real viewport-pixel clicks:
  `wb_sketch/tests/interaction.rs` (reuse its `Harness`).
- Full-stack sketch→feature→OCCT-solid pipelines:
  `kernel_occt/tests/part_design_stack.rs` (dev-deps on wb_part/wb_sketch).
- Solver/geometry math is unit-tested next to the code. Assert geometric
  properties (bounds, tangency, closure), not implementation details.
- Before committing: fmt, clippy (zero warnings), full test suite, and a
  short `cargo run` smoke check watching for `printcad.vulkan` output.

## Known approximations / roadmap

- Faces are identified geometrically (coplanar triangle regions), not
  topologically — coplanar-but-disjoint faces select together. OCCT face/edge
  ids through the render mesh is the next big unlock (solid fillet/chamfer,
  true sketch-on-face references).
- "Through all" pocket = ±1e5 mm symmetric cut (`THROUGH_ALL_MM`).
- `orientation_cube/mod.rs` (1340 LOC) still needs the camera-style split.
