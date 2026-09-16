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
cargo test --workspace            # full suite (~390 tests)
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
- `ui_kit` — the design system, below the workbenches so their panel code
  can use it (behind their `egui` feature): `tokens` (the palette and
  size constants, named after the design's variables), `theme`
  (`apply_theme`, bundled IBM Plex Sans/Mono under `fonts/`, fetched by
  `scripts/vendor-fonts.sh`; `sans/sans_medium/sans_semibold/mono` font
  helpers), `widgets` (Card, overline, badge, key chip, toggle, check row,
  note card, the button set, `tool_button`, `section_header`, `QtyField`,
  `select_field`, `PrefRow` + `pref_group`, `planned`), `icon` (the SVG
  set under `icons/`, vendored by `scripts/vendor-icons.mjs` into a
  generated `icon_table.rs`; `icon::texture/draw` rasterize with a
  font-free usvg — the system-font scan is far too slow for 200 icons;
  `select.svg` and `expression.svg` are hand-authored locals). A test
  fails when the table and the directory disagree.
- `workbenches/wb_sketch` — sketcher: `tools.rs` + `tools/{draw,modify,
  transform}.rs` (state machine), `geom2d.rs` (intersection/sampling math),
  `snap.rs`, `solver.rs` (LM, uniform constraint records + diagnostics),
  `profile.rs` (closed-wire extraction), `overlay.rs` (screen-space rendering
  while editing), `glyphs.rs` (constraint icons and dimension layouts),
  `constrain.rs` (which constraint a toolbar action creates for the
  selection's shape), `panel.rs` (the task panel), `style.rs` (icons and
  names per element/constraint kind).
- `workbenches/wb_part` — Pad/Pocket/Revolution/Groove/Loft/Pipe/Helix/
  Primitive/Hole/Fillet/Chamfer/Draft/Thickness/patterns/Boolean features
  (`feature.rs`), per-feature panel editors (`editors.rs`), the task
  lifecycle (`task.rs`: snapshot on open, live edits, Cancel restores or
  deletes a tool-created feature); `build.rs` translates a body's feature
  history into kernel `SolidOp` chains (`BuildPlan` maps op index → feature
  for error attribution).
- `render_vk` — data-only renderer (`FrameSubmission` in, pixels out). GPU
  picking with async readback; per-body mesh cache keyed by (id, revision).
- `app_shell` — binary. `app/` modules: `frame.rs` (per-frame loop),
  `input.rs` (events, selection), `commands.rs` (UI command application),
  `recompute.rs` (parametric rebuild driver), `workbench_host.rs` (ctx
  plumbing), `kernel_worker.rs` (kernel thread, keeps the UI responsive).
  `ui/` is one module per region: `menu_bar`, `toolbar` (rows from
  `ToolDescriptor.row`, variant dropdowns), `combo_view` (tree +
  `property_panel`), `feature_tree`, `task_panel` (host of the workbench
  task; OK/Cancel/Enter/Esc), `status_bar`, `view_toolbar` (floating
  pill), `hud` (workbench HUD corners, OVP card, hover card), `overlays`
  (line/mark/label painters), `start_page`, `preferences` (modal on a
  draft `UserSettings`, committed by `CommitSettings`), `command_palette`
  (Ctrl+K), `step_import_modal`, `log_view`, `host_ctx`.

The `Workbench` trait's UI surface: `configure` registers
`ToolDescriptor`s (icon, row, category, variants, `planned` note);
`is_tool_enabled`/`tool_toggled` decide button state each frame; `task()` +
`ui_task_panel()` own the right panel (`TaskRequest` in, `TaskOutcome` out);
`viewport_hud()`, `status_items()`, `editing_feature()`,
`get_screen_space_overlays/marks/labels()` feed the viewport and chrome;
`ui_settings()` draws the bench's Preferences page. Colors reach the
workbenches through `WorkbenchRuntimeContext.sketch_palette`, never as
literals.

**Placeholders.** The design shows Part Design and Sketcher elements the app
does not implement yet. They stay on screen as disabled controls with a
`// PLANNED: <what it does when built>` comment next to them and a
`ToolDescriptor::planned(note)` on tools (`tool_button` renders them dim,
the host never dispatches a planned id). Nothing outside those two
workbenches gets a placeholder.

**Tasks and undo.** A feature edit is a task in the right panel: edits apply
live, OK accepts, Cancel writes the opening snapshot back (or deletes the
feature the tool just created). `frame.rs` skips the per-frame
`journal.note` while a task is open, so one task is one undo entry;
`TaskClosed` closes the gesture.

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
is logged structured at import. **Announcement discipline:** `progress::
context` marks a *phase* and resets the display — call it once per phase,
never per body or per face. Anything emitted inside a loop is
`progress::detail` (a kernel-style sub-stage: shown under a sequential
phase, ignored while our counted `stage_at` loop owns the display) or
`stage_at`. Our counted stage speaks alone — the kernel's per-body stages
from twenty threads are noise, not information; the 1 s frame log's
`status_changes` counts status-text changes per second (a readable bar
changes ~1/s; a slot overwritten by threads changes every frame).

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
  *declaration* order): renderer before window. Do not reorder. Inside the
  renderer the same rule bites: anything that frees device objects in its own
  `Drop` (the egui renderer) must be `take()`n and dropped in
  `RendererCore::drop` BEFORE `destroy_device`, or it runs on a dead device —
  a hang or segfault at exit plus a wall of "leaked objects".
- **Vulkan validation layers default to debug builds only**
  (`RenderSettings::default`); `PRINTCAD_VULKAN_VALIDATION=1` enables them for
  a release run. `PRINTCAD_EXIT_AFTER_MS` quits through the real exit path
  after a delay (keeps the loop awake) so teardown can be timed on a loaded
  document.
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
rendering. `PRINTCAD_OPEN_FILE` / `PRINTCAD_OPEN_DOC` /
`PRINTCAD_BENCH_ORBIT` / `PRINTCAD_EDGE_MIN_PX` / `PRINTCAD_NO_EDGES` are
bench hooks (frame.rs, mesh.rs); `PRINTCAD_BENCH_SKETCH=1` opens a
constrained sketch for editing and `=pad` pads it and opens the Pad task.
Any of these skips the start page. The 1 s `printcad.frame` log reports
fps + phase costs while frames are being produced.

**The 3D scene is cached between changes.** The scene pass resolves into a
persistent scene image and runs only when `scene_fingerprint(frame)`
(`render_vk/src/core.rs`) changes; every frame copies that image under the
UI pass. UI-only frames (hover, panels, typing) therefore cost ~2 ms on any
model. **Completeness of the fingerprint is the contract**: anything the
scene pass reads — camera, viewport, lighting, per-body id/revision/mesh
pointer/color/highlight/wireframe, edge suppression — must be hashed there,
or a change shows stale. The status bar shows both numbers because they are
two things: `FPS` (UI frames presented) and `scene: N/s` (scene redraws;
"cached" when zero). `PRINTCAD_BENCH_SPIN` keeps the loop awake with no
scene change (expect zero redraws); `PRINTCAD_BENCH_ORBIT` expects one per
frame.

## Interaction model (current bindings)

MMB drag = orbit (MMB click = pivot pick) · RMB drag = pan · wheel = zoom ·
LMB = select (click sketch → tree-select; click solid → face-first, double
click → whole body; LMB drag in sketch = box select; ctrl = additive).
While editing a sketch the view is locked planar (orbit + cube rotation
disabled; pan/zoom/roll allowed). The window opens on the start page
(`Screen::Start`); the recent list lives in `settings::recent`.

## UI conventions

- `ui_kit::tokens` and the font helpers are the only source of colors and
  sizes in UI code: no color literals in panels (converting a palette or
  settings color to `Color32` is fine).
- Every icon is named in `ToolDescriptor::icon` or drawn through
  `ui_kit::icon`; each workbench's tests assert every registered tool's
  icon (and variant icon) exists. New icons go through `scripts/vendor-icons.mjs`, never by hand
  (the script strips metadata and normalises the stroke color).
- UI-local state (filters, drafts, palette query) lives on `UiLayer`;
  anything the host mutates is seeded from `UiFrameInputs` every frame.
- `ui-mockup/` is the design reference and stays untracked; only tokens,
  icons and fonts are vendored from it.

## Testing conventions

- Sketcher end-to-end tests drive `on_input` with real viewport-pixel clicks:
  `wb_sketch/tests/interaction.rs` (reuse its `Harness`).
- Full-stack sketch→feature→solid pipelines:
  `kernel_ogeom/tests/part_design_stack.rs` (dev-deps on wb_part/wb_sketch).
- Solver/geometry math is unit-tested next to the code. Assert geometric
  properties (bounds, tangency, closure), not implementation details.
- Before committing: fmt, clippy (zero warnings), full test suite, and a
  short `cargo run` smoke check watching for `printcad.vulkan` output. For
  UI work, a headless capture of the release build (`PRINTCAD_BENCH_SKETCH=pad
  PRINTCAD_EXIT_AFTER_MS=…`, `grim`, `ydotool` for keys) is the smoke
  check; verify on a small STEP, never the huge assembly files.
- Comments describe present behaviour, never the change that produced it.
  `node scripts/lint-comment-rot.mjs --all` gates this in CI (default mode
  lints only lines added against `origin/master`; `--pedantic` adds an
  advisory tier). No "used to", "no longer", "since X landed", and no issue,
  PR or commit references in comments — the one sanctioned place for a
  kernel issue number is the `#[ignore = "kernel: … (ogeom-rs#N)"]` string.
  `lint-comment-rot: ignore` on a line opts it out when a reference is
  load-bearing. The tracked pre-commit hook runs it in `--staged` mode over
  the lines a commit adds; enable once per clone with
  `git config core.hooksPath .githooks`.

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
