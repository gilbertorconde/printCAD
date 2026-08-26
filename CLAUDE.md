# printCAD — agent notes

Linux-native parametric CAD app aimed at FDM/SLA printing.
Rust workspace + Vulkan (ash) + egui + the pure-Rust ogeom B-rep kernel
(github.com/gilbertorconde/ogeom-rs, pinned by rev in `Cargo.toml`).

**Never name the tools or systems this project draws on** — FreeCAD, X11,
or any other reference point — anywhere in the project: no code, comments,
identifiers, file or folder names, docs, UI strings, or commit messages.
Describe conventions and architectures on their own terms, not by
attribution. Naming an actual platform requirement (e.g. the display systems
the app runs on) is fine; naming an inspiration is not. (This paragraph is
the single sanctioned mention.)

## Commands

```bash
cargo run -p app_shell            # launch the app (needs Vulkan + Wayland/X11)
cargo run --release -p app_shell  # for real STEP files — see the profile note
cargo test --workspace            # full suite (~200 tests)
cargo clippy --workspace --all-targets   # CI enforces -D warnings
cargo fmt --all                   # CI enforces --check
```

- No system CAD libraries needed — the ogeom kernel is pure Rust, pulled as a
  pinned git dependency (bump the rev in the workspace `Cargo.toml`; a
  commented `[patch]` there points at a local checkout for kernel dev).
- STEP tests use the bundled fixture
  `crates/kernel_ogeom/tests/data/box_native.step`; set
  `PRINTCAD_TEST_STEP_FILE` to test against a richer model. (`box.step` is an
  OCCT-flavoured file kept for the ignored SURFACE_CURVE interop test.)
- `[profile.dev.package."*"] opt-level = 3` in the workspace `Cargo.toml` is
  load-bearing, not tidiness: the kernel is numeric code and runs ~26x slower
  unoptimized, which made a large STEP import look like a hang. Our own crates
  stay unoptimized (fast rebuilds, readable backtraces), so a debug build is
  still ~1.4x slower than release — use `--release` when timing anything.
- `crates/kernel_ogeom/examples/import_bench.rs` prints the phase breakdown of
  an import; reference timings live in the import-performance memory.
- Vulkan validation layers, when installed, are routed into `tracing`
  (target `printcad.vulkan`). Keep the app validation-clean.

## Crate map / dataflow

- `kernel_api` — pure data contract (TriMesh, ProfileWire w/ ellipse+B-spline
  segments, `SolidOp` = sweep/loft/pipe/primitive/dress-up/transform/boolean,
  ExtrudeTermination, TessellationSettings, ChainError). No geometry code.
- `kernel_ogeom` — pure-Rust kernel adapter. STEP import builds bodies from
  the document's **placed occurrences** (`Document::occurrences_of`), never
  from `import.solids` — the latter are part-local, so an assembly built from
  them puts every part at its own origin. The node walk mirrors the kernel's
  preorder flatten so the n-th part leaf is the n-th body. (`import.rs`) +
  `execute_solid_chain` (`chain.rs`): one in-memory ogeom `Model` per chain,
  the running `Shape` threaded op-by-op (`ops/{sweep,primitive,dressup,
  loft_pipe,pattern}.rs`); native-format text blobs only at the boundaries
  (result out, `SolidOp::Boolean` tool in, via `io::native`). Errors carry the
  failing op index (`ChainError`). Profile wires group by containment:
  nested = holes, disjoint = separate solids (compounded when regions stay
  disjoint). Patterns re-run the tool op under the transform rather than
  instancing. Tests marked `#[ignore]` document kernel-side gaps — grep for
  `kernel:` in `tests/` before assuming a feature is wired wrong.
- `core_document` — Document (feature tree DAG, bodies w/ `tip`, tar `.prtcad`
  persistence), `Workbench` trait + runtime context, snapshot undo
  (`undo.rs`), workbench registry (`service.rs`), core datums (`datum.rs`:
  plane/line/point + attachment + offset, shared across workbenches).
- `doc_server` — the document server: `printcad-serverd` binary +
  `DaemonClient`/`DirectFiles` implementations of the `DocumentServer` trait;
  length-prefixed JSON frames (`framing.rs`), integration-tested against the
  real spawned daemon (`tests/daemon.rs`).
- `workbenches/wb_sketch` — sketcher: `tools.rs` + `tools/{draw,modify,
  transform}.rs` (state machine), `geom2d.rs` (intersection/sampling math),
  `snap.rs`, `solver.rs` (LM, uniform constraint records + diagnostics),
  `profile.rs` (closed-wire extraction), `overlay.rs` (screen-space rendering
  while editing).
- `workbenches/wb_part` — Pad/Pocket/Revolution/Groove/Loft/Pipe/Helix/
  Primitive/Hole/Fillet/Chamfer/Draft/Thickness/patterns/Boolean features
  (`feature.rs`), per-feature panel editors (`editors.rs`); `build.rs`
  translates a body's feature history into kernel `SolidOp` chains
  (`BuildPlan` maps op index → feature for error attribution).
- `render_vk` — data-only renderer (`FrameSubmission` in, pixels out). GPU
  picking with async readback; per-body mesh cache keyed by (id, revision).
- `app_shell` — binary. `app/` modules: `frame.rs` (per-frame loop),
  `input.rs` (events, selection), `commands.rs` (UI command application),
  `recompute.rs` (parametric rebuild driver), `workbench_host.rs` (ctx
  plumbing), `kernel_worker.rs` (kernel thread, keeps the UI responsive).

Recompute loop: workbench edits document → features marked dirty via the
dependency DAG → `drive_part_recompute` (each frame) builds `SolidOp` chains →
kernel worker thread → results land in the document's imported-geometry
sidecar → rendered/picked like any body.

Import performance: the per-solid work and each mesh's face pass go through
`ogeom_core::parallel::map_ordered` (order-preserving, so output is identical
at any thread count — `tests/step_import.rs` asserts that). Never nest two
`map_ordered` passes: `tess::Faces::{Wide, Inline}` says which level owns the
threads. Import meshes inline from the model already in memory; a deferred
pass would have to parse every snapshot back, which cost more than the
meshing. `crates/kernel_ogeom/examples/import_bench.rs` reports the phase
breakdown — measure with it before optimizing. STEP text is decoded lossily
(exporters emit Latin-1 in string literals).

Progress/cancel: the worker installs one `Watch` per job
(`kernel_ogeom::{Watch, watched, Canceller}`, re-exported so app code never
depends on `ogeom` directly). `kernel_ogeom::progress::context` announces our
own labels prefixed with `CONTEXT_PREFIX`; ogeom announces its own stages on
the same thread-local channel. The worker's sink files them into a shared
`Activity` slot (not a channel — `mpsc::Sender` is `Send` but not `Sync`), the
status bar reads it each frame, and `Canceller` backs the Cancel button. Our
op/face/solid loops call `progress::checkpoint()` themselves, since
`triangulate_face` has no checkpoints of its own. Stages announcing
`(done, total)` — kernel-side, or ours via `progress::stage_at` fed by a
shared monotone counter in the parallel import loop — draw as a determinate
bar instead of the spinner; a new context resets counts to unknown.
`report.untrimmed_faces` (STEP entity ids of faces that will draw with gaps)
is logged structured at import.

## Kernel gap protocol

The geometry kernel (ogeom) is developed by the project owner in its own
repo. When a feature needs a kernel capability that ogeom lacks — missing
API, refusal, wrong result — do NOT paper over it app-side (no mesh-level
hacks, no silently degraded feature). Instead:

1. Wire the op anyway; let the kernel's refusal surface as a clean
   `ChainError` on the owning feature.
2. Add a test for the intended behavior marked
   `#[ignore = "kernel: <precise reason>"]` so it flips green when the fix
   lands.
3. **File an issue on the kernel repo**
   (`gh issue create -R gilbertorconde/ogeom-rs`) with the desired
   API/signature, its semantics, a minimal repro in ogeom API terms, and an
   acceptance test — and reference the issue number in the test's ignore
   reason (`kernel: ... (ogeom-rs#N)`).
4. When the fix lands: bump the ogeom rev pin in the workspace `Cargo.toml`,
   un-ignore the matching tests, rerun the full suite.

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
- **Every user-edit mutator on `Document` records exactly one op; derived
  state never does.** Mutators follow validate → resolve (ids, timestamps,
  seq) → build `DocumentOp` → `apply_op` → record; replay runs the same code
  live edits ran (`core_document/src/op.rs`, tests in `tests/op_replay.rs`).
  Dirty flags, recompute errors and imported-geometry sidecars are per-replica
  consequences, excluded from the replicated projection. The outbox is
  `#[serde(skip)]` and Clone-EMPTIES — snapshots carry state, never pending
  ops. Never add a `&mut` escape hatch to `Document`; capture is only total
  because none exists.
- **The app is a client of a document server** (`core_document/src/server.rs`
  trait = the wire protocol; `crates/doc_server` has the `printcad-serverd`
  daemon — one per session, unix socket under `$XDG_RUNTIME_DIR/printcad`,
  single client, exits on disconnect — plus the `DirectFiles` fallback). The
  daemon stores opaque `.prtcad` bytes and op envelopes
  (`<file>.oplog.jsonl`), never deserializing a `Document`. Saves cross as
  client-serialized bytes with `at_seq`; `mark_clean()` only fires if
  `at_seq` still equals `mutation_seq` on completion. Undo/redo/new/open send
  `Rebase`. **Every exit path must call `wait_for_document_saves()`** (which
  flushes the server) or a write in flight is abandoned mid-file.
  `PRINTCAD_SERVERD` overrides the daemon binary for dev.
- **`FeatureNode.seq` is THE build-history ordering key.** `created_at` has
  millisecond ties that order randomly — never sort history by it.
- **Undo is per-user inverse ops, not snapshots** (`core_document/src/
  history.rs`). Each mutator computes its inverse from pre-apply state;
  gestures close at journal boundaries (mouse-up frames, labeled commands).
  Undo/redo apply ordinary forward ops — they flow to the server and peers,
  never send `Rebase`, and never replace the document. Non-invertible ops
  (imports, asset adds) are barriers that clear history. Coalescing keeps
  the LAST op with the FIRST inverse. `Document::clone` still preserves the
  `#[serde(skip)]` sidecars (save snapshots depend on it; `undo.rs` tests
  pin it). Solids stay derived: `after_history_jump` re-marks part features
  dirty.
- Kernel shapes are plain `Send + Sync` data; tests run in parallel with no
  serialization mutex. The kernel-worker thread exists for UI responsiveness,
  not safety.
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
- Persisted shape blobs (`brep/<uuid>.bin` in `.prtcad`, `SolidOp::Boolean`
  tools) are ogeom native-format text ("ogeom" magic); pre-migration blobs are
  dropped on load with a warning.

## Render loop

Frames are rendered **on demand**, not continuously: a frame is scheduled
while input is fresh (150 ms tail), async work is pending (kernel jobs,
document open/save, file dialog, deferred import), the camera tween or bench
orbit is running, the last frame was drawn edge-suppressed (one restore
frame), or egui asked for a repaint (`repaint_delay` == 0; finite delays
become `WaitUntil`, e.g. caret blink). Otherwise the event loop sleeps in
`ControlFlow::Wait` until the next OS event. Consequences: anything that
completes on a background channel must be covered by one of the
"work pending" flags or it will not surface until the next input event, and
`fps_cap` now caps the *active* rate rather than implying continuous
rendering. `PRINTCAD_OPEN_FILE` / `PRINTCAD_BENCH_ORBIT` /
`PRINTCAD_EDGE_MIN_PX` / `PRINTCAD_NO_EDGES` are bench hooks (frame.rs,
mesh.rs); the 1 s `printcad.frame` log reports fps + phase costs while
frames are being produced.

## Interaction model (current bindings)

MMB drag = orbit (MMB click = pivot pick) · RMB drag = pan · wheel = zoom ·
LMB = select (click sketch → tree-select; click solid → face-first, double
click → whole body; LMB drag in sketch = box select; ctrl = additive).
While editing a sketch the view is locked planar (orbit + cube rotation
disabled; pan/zoom/roll allowed).

## Testing conventions

- Sketcher end-to-end tests drive `on_input` with real viewport-pixel clicks:
  `wb_sketch/tests/interaction.rs` (reuse its `Harness`).
- Full-stack sketch→feature→solid pipelines:
  `kernel_ogeom/tests/part_design_stack.rs` (dev-deps on wb_part/wb_sketch).
- Solver/geometry math is unit-tested next to the code. Assert geometric
  properties (bounds, tangency, closure), not implementation details.
- Before committing: fmt, clippy (zero warnings), full test suite, and a
  short `cargo run` smoke check watching for `printcad.vulkan` output.

## Known approximations / roadmap

- Faces are identified geometrically (coplanar triangle regions), not
  topologically — coplanar-but-disjoint faces select together. Dress-up edge
  selection and up-to-face terminations therefore reference faces by a sample
  point + normal (`FacePick`), re-resolved against the current solid each
  rebuild. Kernel face/edge ids through the render mesh is still the next
  big unlock (per-edge picking, true sketch-on-face references).
- "Through all" derives its length from the base solid's bounding box; up-to-
  face trims with a half-space, so only PLANAR target faces terminate exactly
  (curved to-first/to-last faces stop at the profile-centroid hit distance).
- Helix with height 0 (flat spiral) is rejected; use a small pitch instead.
- Hole threads are standards data only (tap-drill / ISO 273 clearance
  diameters); no helical thread geometry is generated.
- `orientation_cube/mod.rs` (1340 LOC) still needs the camera-style split.
