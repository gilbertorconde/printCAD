# printCAD: agent notes

Linux-native parametric CAD app aimed at FDM/SLA printing.
Rust workspace + wgpu (Vulkan, Metal, DirectX 12) + egui + the pure-Rust ogeom B-rep kernel
(a git dependency on its repository pinned by `rev` in the workspace
`Cargo.toml`, moved to each main commit that closes one of this
project's kernel issues; back to a crates.io requirement at its
releases).

**Never name the tools or systems this project draws on** (FreeCAD, X11,
or any other reference point) anywhere in the project: no code, comments,
identifiers, file or folder names, docs, UI strings, or commit messages.
Describe conventions and architectures on their own terms, not by
attribution. Naming an actual platform requirement (e.g. the display systems
the app runs on) is fine; naming an inspiration is not. The one exception
is the README's closing Inspiration section, which lists them with links;
keep them there and nowhere else. (This paragraph is the single sanctioned
mention outside that section.)

## Commands

```bash
cargo run -p app_shell            # launch the app (needs a Vulkan, Metal or DX12 GPU)
cargo run --release -p app_shell  # for real STEP files; see the profile note
cargo test --workspace            # full suite (~1900 tests)
node scripts/test-budget.mjs      # the same, timed against its budget (what CI runs)
node scripts/test-budget.mjs --changed   # only what a change reaches, its own crates first
node scripts/test-lines.mjs       # tests' share of the code, held under a ceiling in CI
cargo clippy --workspace --all-targets   # CI enforces -D warnings
cargo fmt --all                   # CI enforces --check
node scripts/gates.mjs            # every gate above in order, stopping at the first failure (--all for the full suite)
                                  # (it also clippies each bench --lib without egui, the headless build its dependents use)
node scripts/check-layers.mjs     # each crate depends only on crates of lower layers (the table in the script)
cargo test -p <crate>             # one crate while iterating; clippy and fmt take -p too
```

- The document server is a separate binary the app spawns from its own
  directory, so a release run needs `cargo build --release` for the whole
  workspace: `-p app_shell` alone leaves `target/release/printcad-serverd`
  missing and the app falls back to direct file I/O with a warning.
- `[profile.dev.package."*"] opt-level = 3` in the workspace `Cargo.toml` is
  load-bearing, not tidiness: the kernel is numeric code and runs ~26x slower
  unoptimized, which made a large STEP import look like a hang. Our own crates
  stay unoptimized (fast rebuilds, readable backtraces), so a debug build is
  still ~1.4x slower than release, so use `--release` when timing anything.
- wgpu's validation errors are routed into `tracing` (target
  `printcad.gpu`), with the graphics API's own validation under them where
  it is installed. Keep the app validation-clean.
- Before committing: fmt, clippy (zero warnings), full test suite, and a
  short `cargo run` smoke check watching for `printcad.gpu` output. For
  UI work, a headless capture of the release build (`PRINTCAD_BENCH_SKETCH=pad
  PRINTCAD_EXIT_AFTER_MS=…`, `grim`, `ydotool` for keys) is the smoke
  check; verify on a small STEP, never the huge assembly files.
- Comments describe present behaviour, never the change that produced it.
  `node scripts/lint-comment-rot.mjs --all` gates this in CI (default mode
  lints only lines added against `origin/master`; `--pedantic` adds an
  advisory tier). No "used to", "no longer", "since X landed", and no issue,
  PR or commit references in comments; the one sanctioned place for a
  kernel issue number is the `#[ignore = "kernel: … (ogeom-rs#N)"]` string.
  `lint-comment-rot: ignore` on a line opts it out when a reference is
  load-bearing. The tracked pre-commit hook runs it in `--staged` mode over
  the lines a commit adds; enable once per clone with
  `git config core.hooksPath .githooks`.

## Crate map / dataflow

Layers, lowest first (`scripts/check-layers.mjs`); each crate's full notes
are in the skill the index below names.

- Foundations: `sketch_solver`, `image_edges`, `metrology` (std-only maths),
  `viewport_camera`, `transform_gizmo`, `axes`, `feature_walk`, `local_ipc`,
  `project_archive`, `ui_kit` (design system), `kernel_api` (pure data
  contract: `TriMesh`, `ProfileWire`, `SolidOp`, `ChainError`).
- Services: `bench_api` (package contract, wasm32-wasip2), `surface_texture`,
  `kernel_ogeom` (the only crate on ogeom), `render_wgpu` (data-only:
  `FrameSubmission` in, pixels out), `settings`, `agents` (agent client, MCP).
- `core_document`: `Document` (feature tree DAG, bodies with `tip`, tar
  `.prtcad`), ops and undo, the `Workbench` trait and registry, datums, the
  command contract; above it `bench_panel` (declared panels in egui).
- On the document: `doc_server` (`printcad-serverd`), `doc_render` (CPU
  thumbnails, PDF, print layout), `scripting` (Lua), benches `wb_sketch`,
  `wb_assembly`, `wb_inspect`, `wb_mesh`, `wb_wasm` (packages); then
  `wb_design`, `wb_surface` on the sketcher's; then `bench_fixtures`,
  `project_tools`; the `workbenches` facade; `app_shell` (`printcad`).

The host never names a bench: everything crosses the `Workbench` trait
(`scripts/check-workbench-seam.mjs`). Recompute loop: workbench edits
document → features marked dirty via the dependency DAG →
`drive_part_recompute` (each frame) asks every bench for its `rebuild_jobs`
→ kernel worker thread → results land in the document's imported-geometry
sidecar → rendered/picked like any body.

## Kernel gap protocol

The geometry kernel (ogeom) is developed by the project owner in its own
repo. When a feature needs a kernel capability that ogeom lacks (missing
API, refusal, wrong result), do NOT paper over it app-side (no mesh-level
hacks, no silently degraded feature). Instead:

1. Wire the op anyway; let the kernel's refusal surface as a clean
   `ChainError` on the owning feature.
2. Add a test for the intended behavior marked
   `#[ignore = "kernel: <precise reason>"]` so it flips green when the fix
   lands.
3. **File an issue on the kernel repo**
   (`gh issue create -R gilbertorconde/ogeom-rs`) with the desired
   API/signature, its semantics, a minimal repro in ogeom API terms, and an
   acceptance test, and reference the issue number in the test's ignore
   reason (`kernel: ... (ogeom-rs#N)`).
4. When the fix lands: move the workspace `Cargo.toml`'s `ogeom` `rev` to
   the main commit that closes it (or the version requirement, at a
   release), un-ignore the matching tests, rerun the full suite.

## Invariants: violate these and things break subtly

- **`app/gfx.rs` field order IS the teardown contract** (struct fields drop in
  *declaration* order): renderer before window. Do not reorder: the
  surface is made from the window's raw handles (`create_surface_unsafe`)
  and must go first.
- **GPU validation defaults to debug builds only**
  (`RenderSettings::default`); `PRINTCAD_GPU_VALIDATION=1` (or
  `PRINTCAD_VULKAN_VALIDATION=1`) enables it for a release run. `PRINTCAD_EXIT_AFTER_MS` quits through the real exit path
  after a delay (keeps the loop awake) so teardown can be timed on a loaded
  document.
- **UiLayer must never own state the host mutates.** `active_tool` and
  `active_workbench` are seeded from `UiFrameInputs` every frame. A parallel
  copy in the UI caused an infinite New-Body loop once. A bench talks back
  to the host only through `ctx.request(HostRequest)` (tool, selection,
  journal label, a tool gesture's step closed or undone (`CloseToolGesture`,
  `UndoToolGesture`), bench switch or start-on, camera, finish editing, text
  for the clipboard (`CopyText`, a package's `Request::CopyText`), or
  another bench's command by id, `RunCommand`, run and recorded as a
  script's call) and
  `ctx.active_document_object`; `HookOutcome::take` collects them and
  `apply_hook_outcome` applies every one, from every hook site (a panel
  hook's arrive as `UiCommand::HostRequest`; a lifecycle hook's switch
  requests are the one thing dropped, since it runs inside a switch).
- **Adding a UI action** = one variant in `ui/commands.rs` + one arm in
  `app/commands.rs::fold_command`, which acts at once or folds it into a
  `FrameIntents` field that `app/commands/intents.rs` applies in its fixed
  order (two-phase dispatch preserves ordering). A change to settings alone
  needs no variant: the UI pushes `UiCommand::EditSettings` with a
  `SettingsEdit` (`SettingsEdit::set(|s| &mut s.field, value)`), applied in
  phase 1 and saved with the frame's other settings changes. `UiLayer::run`
  (`ui/frame.rs`) draws one function per region, each pushing its commands
  in the order it draws.
- **Every user-edit mutator on `Document` records exactly one op; derived
  state never does.** Mutators follow validate → resolve (ids, timestamps,
  seq) → build `DocumentOp` → `apply_op` → record; replay runs the same code
  live edits ran (`core_document/src/op.rs`, tests in `tests/document/op_replay.rs`).
  Dirty flags, recompute errors and imported-geometry sidecars are per-replica
  consequences, excluded from the replicated projection. The outbox is
  `#[serde(skip)]` and Clone-EMPTIES: snapshots carry state, never pending
  ops. Never add a `&mut` escape hatch to `Document`; capture is only total
  because none exists.
- **The app is a client of a document server, one connection per tab**
  (`core_document/src/server.rs` trait = the wire protocol; `crates/doc_server`
  has the `printcad-serverd` daemon (one per document, unix socket under
  `$XDG_RUNTIME_DIR/printcad`, any number of clients, relaying ops among
  them, exits when the last leaves) plus the
  `DirectFiles` fallback). The daemon stores opaque `.prtcad` bytes and op
  envelopes (`<file>.oplog.jsonl`), never deserializing a `Document`. Ops
  recorded before a document has a file go to `unhomed-<socket hash>.oplog.jsonl`,
  keyed by the socket, and an untitled tab's socket carries the tab's id
  (`socket_path_for_untitled(tab)`), so two unsaved documents never share
  a log; one file for all of them meant the first to save took the other's
  history. The outbox goes to the server at the end of each frame, but
  the tab on screen keeps it while a mouse button is held, so a drag's
  whole-payload writes reach the server and peers as one op on release
  (`DocumentSession::send_outbox_after_frame`); everything that ends a
  tab's document or connection (save, open, close, a server switch,
  `wait_for_document_saves`) calls `send_outbox` first. Saves cross as
  client-serialized bytes with `at_seq`;
  `mark_clean()` only fires if `at_seq` still equals `mutation_seq` on
  completion. Undo/redo/new/open send `Rebase`. **Every exit path must call
  `wait_for_all_document_saves()`** (every tab, each flushing its server)
  or a write in flight is abandoned mid-file.
  `PRINTCAD_SERVERD` overrides the daemon binary for dev.
- **`FeatureNode.seq` is THE build-history ordering key.** `created_at` has
  millisecond ties that order randomly; never sort history by it.
- **Undo is per-user inverse ops, not snapshots** (`core_document/src/
  history.rs`). Each mutator computes its inverse from pre-apply state;
  gestures close at journal boundaries (mouse-up frames, labeled commands).
  Undo/redo apply ordinary forward ops: they flow to the server and peers,
  never send `Rebase`, and never replace the document. Non-invertible ops
  (imports, asset adds) are barriers that clear history. Coalescing keeps
  the LAST op with the FIRST inverse. `Document::clone` still preserves the
  `#[serde(skip)]` sidecars (save snapshots depend on it; `step_persistence.rs` tests
  pin it). Solids stay derived: `after_history_jump` re-marks part features
  dirty.
- Kernel shapes are plain `Send + Sync` data; tests run in parallel with no
  serialization mutex. The kernel-worker thread exists for UI responsiveness,
  not safety.
- **Sketch endpoint snapping REUSES point ids**: that shared-vertex topology
  is what makes profiles closed for `profile::extract_wires`. Don't create
  coincident duplicate points. The profile is the sketch's closed loops:
  curves with a loose end (a stray line, a spur off a loop) are pruned away
  and loops enclosing no area dropped; it fails only when nothing closes
  (`profile::loose_ends` names what was left out). Curves that cross or
  branch go through `regions.rs` instead, whose default (every region, one
  nested in a loop a hole) is the same shape for a sketch that does neither.
- **Pocket/Groove cut AGAINST the sketch normal by default** (a face
  sketch's normal points out of the material, so the default digs in).
- **NDC is Y-down**: the camera bakes a Y flip into `view_proj`, and the
  renderer's vertex shaders flip it back to the GPU's Y-up clip space.
  Transform helpers live in `core_document::runtime` (ctx methods + free
  functions); mirror them, never re-derive with a different convention.
- **Camera orientation is preset-relative** (`q·(−depth)=forward`,
  `q·vertical=up` in the active axis preset, default Z-up). Never build
  orientation quats against a hardcoded XYZ basis.
- Renderer hot path **never waits on the GPU**: picks and pictures are
  buffers mapped asynchronously and collected after a non-blocking poll at
  the top of the next frames; wgpu keeps a replaced buffer alive until the
  GPU is done with it. Keep it that way.
- `serde_json` is built with `float_roundtrip` (workspace): without it a
  JSON number can parse one unit off the double it was written from, and
  `f64` sketch values would not survive a save.
- Serde compatibility: new fields on persisted types (features, sketch) take
  `#[serde(default)]` so old `.prtcad` files keep loading.
- A `.prtcad`'s meshes are entries of their own (`mesh/<uuid>.bin`,
  MessagePack; `ImportedGeometry::mesh_path`), the JSON carrying an empty
  mesh in their place (`MESHES_APART` while it is written), since a large
  assembly's meshes as JSON text passed what one buffer may hold on a
  32-bit target; a document whose JSON holds them still loads. A plain
  `.prtcad` is an uncompressed tar (gzip or zstd when its name ends so),
  and every kind reads by its first bytes. Every save writes `meta.json`
  (`core_document::stamp`: the app version `main` names through
  `set_writer_version`, and `CONTAINER_FORMAT`) after the preview and
  before the document; a file without it reads as older (`file_age`). A
  file of a larger `CONTAINER_FORMAT` is refused (`stamp::refusal`); a
  change that alters what old data means raises it and adds its step to
  `stamp::UPGRADES`, which loading runs from the file's format up. The
  app keeps what it learnt on open per tab (`app/file_safety.rs`:
  a newer file's card offering Save As, a read-only file's marks on its
  tab and tree row with Save turned into Save As, the question before
  saving over another version's file, `UserSettings.files`).
- Persisted shape blobs (`brep/<uuid>.bin` in `.prtcad`, `SolidOp::Boolean`
  tools) are ogeom native-format text ("ogeom" magic); pre-migration blobs are
  dropped on load with a warning.

## Load these skills when...

Project skills under `.claude/skills/` hold everything else this file used to carry. Load the one whose area you touch before editing:

- `printcad-document-model`: Document, ops, persistence, doc_server, doc_render, linked copies, notes, placement, formulas, tasks and undo.
- `printcad-kernel`: kernel_api / kernel_ogeom, rebuilds (recompute loop, preview), imports, progress, shape health, face naming, known approximations.
- `printcad-sketcher`: sketch_solver, wb_sketch, image_edges.
- `printcad-benches`: wb_design, wb_surface, wb_assembly, wb_mesh (+ surface_texture), wb_inspect (+ metrology), project import.
- `printcad-workbench-api`: the `Workbench` trait, bench_api, wb_wasm packages and sdk/, the facade and fixtures.
- `printcad-app-shell`: app_shell tabs/state/module map, export, slicer, print layout, clipping plane, measure, placeholders.
- `printcad-ui-conventions`: ui_kit, bench_panel, colors/icons/UiLayer rules, drag limits.
- `printcad-interaction-model`: mouse and keyboard bindings, selection and picking, tree as history, keymap, camera, gizmo, 6-DoF.
- `printcad-render-loop`: on-demand frames, render_wgpu, edge pass cost, the cached scene and its fingerprint.
- `printcad-bench-hooks`: `PRINTCAD_BENCH_*` / `PRINTCAD_OPEN_*` hooks: click-free runs, captures, PR pictures.
- `printcad-scripting-agents`: command contract, Lua scripts, recording, headless `--script`, recipes, agents/MCP/assistant.
- `printcad-testing`: test layout, budget, CI scope, test-lines ceiling, STEP fixtures, PR evidence.
- `printcad-release-site`: release anatomy, packaging, icons, corpus, the GitHub Pages site.
