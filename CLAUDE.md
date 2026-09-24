# printCAD: agent notes

Linux-native parametric CAD app aimed at FDM/SLA printing.
Rust workspace + Vulkan (ash) + egui + the pure-Rust ogeom B-rep kernel
(crates.io, `Cargo.lock` holds the exact version).

**Never name the tools or systems this project draws on** (FreeCAD, X11,
or any other reference point) anywhere in the project: no code, comments,
identifiers, file or folder names, docs, UI strings, or commit messages.
Describe conventions and architectures on their own terms, not by
attribution. Naming an actual platform requirement (e.g. the display systems
the app runs on) is fine; naming an inspiration is not. (This paragraph is
the single sanctioned mention.)

## Commands

```bash
cargo run -p app_shell            # launch the app (needs Vulkan + Wayland/X11)
cargo run --release -p app_shell  # for real STEP files; see the profile note
cargo test --workspace            # full suite (~390 tests)
cargo clippy --workspace --all-targets   # CI enforces -D warnings
cargo fmt --all                   # CI enforces --check
```

- The document server is a separate binary the app spawns from its own
  directory, so a release run needs `cargo build --release` for the whole
  workspace: `-p app_shell` alone leaves `target/release/printcad-serverd`
  missing and the app falls back to direct file I/O with a warning.
- No system CAD libraries needed: the ogeom kernel is pure Rust, released to
  crates.io. Bump it with `cargo update -p ogeom`; a commented
  `[patch.crates-io]` in the workspace `Cargo.toml` points at a local checkout
  for kernel dev.
- 6-DoF input (SpaceMouse and the like) comes from the `sixdof` crate
  (crates.io, this project's own), consumed by version exactly as the kernel
  is, with the same commented `[patch.crates-io]` for local work. It needs no
  system library: it speaks the spacenavd socket
  protocol itself, with the display-server (Magellan) protocol behind its
  `magellan` feature. Nothing is required to build or run without a device.
- STEP tests use the bundled fixture
  `crates/kernel_ogeom/tests/data/box_native.step`; set
  `PRINTCAD_TEST_STEP_FILE` to test against a richer model. (`box.step` is an
  OCCT-flavoured file kept for the ignored SURFACE_CURVE interop test.)
- `[profile.dev.package."*"] opt-level = 3` in the workspace `Cargo.toml` is
  load-bearing, not tidiness: the kernel is numeric code and runs ~26x slower
  unoptimized, which made a large STEP import look like a hang. Our own crates
  stay unoptimized (fast rebuilds, readable backtraces), so a debug build is
  still ~1.4x slower than release, so use `--release` when timing anything.
- `crates/kernel_ogeom/examples/import_bench.rs` prints the phase breakdown of
  an import; reference timings live in the import-performance memory.
- Vulkan validation layers, when installed, are routed into `tracing`
  (target `printcad.vulkan`). Keep the app validation-clean.

## Crate map / dataflow

- `kernel_api`: pure data contract (TriMesh with per-triangle kernel face ids, ProfileWire w/ ellipse+B-spline
  segments, `SolidOp` = sweep/loft/pipe/primitive/dress-up/transform/boolean,
  ExtrudeTermination, TessellationSettings, ChainError). No geometry code.
- `kernel_ogeom`: pure-Rust kernel adapter. STEP and IGES import share one
  path after the read (`import.rs`, reader chosen by extension, `is_iges`);
  IGES solids and closed surface groups become bodies, open sheets are left
  out with a log line. STL, OBJ and 3MF (`mesh.rs`) import as **mesh
  bodies**: triangles and no shape snapshot (`Document::is_mesh_body`),
  welded where normals agree within 30° and outlined at creases and holes;
  they draw, hide and pick and take no features. "Convert to solid" (tree
  and viewport menus) records `RequestMeshSolid`, a history barrier, and
  `drive_mesh_solids` derives the B-rep with the kernel's `solid_from_mesh`
  (coplanar triangles merged into faces; a mesh that does not close becomes
  an open shell, said in the log), after which the body is an ordinary
  imported solid. STEP import builds bodies from
  the document's **placed occurrences** (`Document::occurrences_of`), never
  from `import.solids`; the latter are part-local, so an assembly built from
  them puts every part at its own origin. The node walk mirrors the kernel's
  preorder flatten so the n-th part leaf is the n-th body. (`import.rs`) +
  `execute_solid_chain` (`chain.rs`): one in-memory ogeom `Model` per chain,
  the running `Shape` threaded op-by-op (`ops/{sweep,primitive,dressup,
  loft_pipe,pattern}.rs`); native-format text blobs only at the boundaries
  (result out, `SolidOp::Boolean` tool in, via `io::native`). Errors carry the
  failing op index (`ChainError`). Profile wires group by containment:
  nested = holes, disjoint = separate solids (compounded when regions stay
  disjoint). Patterns re-run the tool op under the transform rather than
  instancing. Tests marked `#[ignore]` document kernel-side gaps; grep for
  `kernel:` in `tests/` before assuming a feature is wired wrong.
- `core_document`: Document (feature tree DAG, bodies w/ `tip`, tar `.prtcad`
  persistence), `Workbench` trait + runtime context, snapshot undo
  (`undo.rs`), workbench registry (`service.rs`), core datums (`datum.rs`:
  plane/line/point/coordinate system + attachment + offset, shared across workbenches).
- `doc_server`: the document server: `printcad-serverd` binary +
  `DaemonClient`/`DirectFiles` implementations of the `DocumentServer` trait;
  length-prefixed JSON frames with the container bytes beside them, never
  inside (`framing.rs`, `server::Payload`), since a document as JSON numbers is
  four times its size and overran the frame cap outright, integration-tested against the
  real spawned daemon (`tests/daemon.rs`).
- `ui_kit`: the design system, below the workbenches so their panel code
  can use it (behind their `egui` feature): `tokens` (the palette and
  size constants, named after the design's variables), `theme`
  (`apply_theme`, bundled IBM Plex Sans/Mono under `fonts/`, fetched by
  `scripts/vendor-fonts.sh`; `sans/sans_medium/sans_semibold/mono` font
  helpers), `widgets` (Card, overline, badge, key chip, toggle, check row,
  note card, the button set, `tool_button`, `section_header`, `QtyField`,
  `select_field`, `PrefRow` + `pref_group`, `planned`, and `Tab` +
  `tab_plus`, the one tab every strip draws: documents, chats, the property
  panel's pages, Preferences), `icon` (the SVG
  set under `icons/`, vendored by `scripts/vendor-icons.mjs` into a
  generated `icon_table.rs`; `icon::texture/draw` rasterize with a
  font-free usvg, since the system-font scan is far too slow for 200 icons;
  `select.svg` and `expression.svg` are hand-authored locals). The same
  table carries the `motion-*` drawings (200×200, their own colours, one
  per movement of a 6-DoF mouse), drawn through `icon::drawing`, which
  rasterizes for the size it is shown at rather than the icon size. A test
  fails when the table and the directory disagree.
- `workbenches/wb_sketch`: sketcher: `tools.rs` + `tools/{draw,modify,
  transform}.rs` (state machine), `geom2d.rs` (intersection/sampling math),
  `snap.rs`, `solver.rs` (LM, uniform constraint records + diagnostics),
  `profile.rs` (closed-wire extraction), `overlay.rs` (screen-space rendering
  while editing), `glyphs.rs` (constraint icons and dimension layouts),
  `constrain.rs` (which constraint a toolbar action creates for the
  selection's shape), `panel.rs` (the task panel), `style.rs` (icons and
  names per element/constraint kind).
- `workbenches/fixtures` (`bench_fixtures`): ready-made scenes built from
  the benches' feature types (a dimensioned sketch, padded, pocketed) for
  the app's `PRINTCAD_BENCH_SKETCH` hook and tests; the host composes
  benches only through it, never by naming them. `app/seam_lint.rs` fails
  when a bench crate, id or feature type appears in `app_shell/src`, and CI
  greps for the same.
- `workbenches/wb_part`: Pad/Pocket/Revolution/Groove/Loft/Pipe/Helix/
  Primitive/Hole/Fillet/Chamfer/Draft/Thickness/patterns/Boolean features
  (`feature.rs`; every one that fuses or cuts carries `refine`, which the
  build follows with a `SolidOp::Refine` merging coplanar faces; the
  Preferences switch is only the default for new features, so geometry
  never depends on who rebuilds it), per-feature panel editors (`editors.rs`), the task
  lifecycle (`task.rs`: snapshot on open, live edits, Cancel restores or
  deletes a tool-created feature); `build.rs` translates a body's feature
  history into kernel `SolidOp` chains (`BuildPlan` maps op index → feature
  for error attribution).
- `workbenches/wb_assembly`: joints between bodies (`joint.rs`: Mate of two
  planar anchors with offset/flip, Align of two axes, Angle between two
  planar anchors, starting at the angle they make; anchors kept in each
  body's own frame), the solver (`solve.rs`: bodies in dependency order,
  each by damped least squares in double precision from where it sits,
  after turning its first joint's directions into agreement, or its first
  angle to its value; rings and
  conflicting joints are reported), and task panels for picking,
  joint settings and moving a body by numbers. Solves run inside the
  gesture that made or edited a joint and record ordinary
  `SetBodyPlacement` ops.
- `render_vk`: data-only renderer (`FrameSubmission` in, pixels out). GPU
  picking with async readback; per-body mesh cache keyed by (id, revision).
- `app_shell`: binary. **Tabs:** `app/session.rs` is `DocumentSession`,
  everything the app keeps per document (document, journal, file, camera,
  selection, active bench and tool, server connection, in-flight open/save,
  presence, the STEP modal, the benches' suspended editing state); the active
  one sits on `PrintCadApp.session`, the rest are parked in `tabs` and
  `app/tabs.rs` swaps them (`switch_tab`, `open_tab`, `close_tab_interactive`,
  `ensure_fresh_tab`: New and Open reuse a blank tab). Background tabs get
  their turn through `with_tab`/`for_each_tab` (drains, rebuilds, outbox);
  a kernel response routes by body id or by the tab that asked for the
  import (`import_owner`). **Only a real switch moves bench state**
  (`Workbench::suspend_session`/`resume_session` through the registry); a
  `with_tab` turn must not touch it. `app/` modules: `frame.rs` (per-frame loop),
  `input.rs` (events, selection), `commands.rs` (UI command application),
  `recompute.rs` (parametric rebuild driver), `workbench_host.rs` (ctx
  plumbing), `kernel_worker.rs` (kernel thread, keeps the UI responsive),
  `sixdof.rs` (6-DoF mouse reader thread; holds the puck's current deflection,
  which `camera::apply_device_motion` integrates once per frame).
  `ui/` is one module per region: `menu_bar`, `toolbar` (rows from
  `ToolDescriptor.row`, variant dropdowns), `combo_view` (tree +
  `property_panel`), `feature_tree`, `task_panel` (host of the workbench
  task; OK/Cancel/Enter/Esc), `status_bar`, `view_toolbar` (floating
  pill; in perspective it carries the field of view, dragged or typed,
  which `CameraController::set_field_of_view` changes keeping the framing), `hud` (workbench HUD corners, OVP card, hover card), `overlays`
  (line/mark/label painters), `start_page`, `preferences` (modal on a
  draft `UserSettings`, committed by `CommitSettings`), `command_palette`
  (Ctrl+K), `step_import_modal`, `log_view`, `host_ctx`.

The `Workbench` trait is the only seam between the host and a bench; the
host never names a bench: `app/seam_lint.rs` and a CI grep over
`crates/app_shell/src` (the lint's own token list excluded) refuse any
line that does. `descriptor()` says
what a bench is: `icon`, the `feature_kinds` it claims (the
`FeatureNode::workbench_id` values it presents, edits, renders, picks and
deletes; Part Design claims `core.datum` too; a kind claimed twice fails
registration), and `modal` for an edit-session bench (entering it remembers
the bench to return to; the first non-modal registration is where a new
document lands, `DocumentService::landing_workbench`). The registry answers
by kind (`owner_of`, `feature_info`), so the tree's icon and Kind row, the
hover card, the double-click edit route and the view lock come from the
owning bench's `feature_info`/`locks_view_to_plane`, whichever bench is
active. The UI surface: `configure` registers
`ToolDescriptor`s (icon, row, category, variants, `planned` note);
`is_tool_enabled`/`tool_toggled` decide button state each frame; `task()` +
`ui_task_panel()` own the right panel (`TaskRequest` in, `TaskOutcome` out);
`viewport_hud()`, `status_items()`, `editing_feature()`,
`get_screen_space_overlays/marks/labels()` feed the viewport and chrome;
`ui_settings()` draws the bench's Preferences page (one rail entry per
registered bench); `feature_info`/`passive_geometry`/`pick_feature`/
`delete_feature`/`property_hints` answer for the feature kinds a bench
claims; `rebuild_jobs`/`invalidate_body`/`invalidate_all` drive solids;
`menu_items`/`on_command` add entries to the viewport body menu, tree rows
and the start page's New cards. `docs/WORKBENCH_GUIDE.md` is the
walkthrough. Colors reach the workbenches through
`WorkbenchRuntimeContext.sketch_palette`, never as literals.

**Placeholders.** Everything the design shows is built. The sketcher's
external geometry (`external.rs`) takes solid edges picked while its tool
is armed (clicks fall through to the host's edge picking), projects each
through `ctx.kernel` (`kernel_api::KernelQueries`, the host hands benches
`kernel_ogeom::QUERIES`) onto the sketch plane in the edge body's frame,
and stores the result as geometry marked in `Sketch::external` with its
`ExternalSource`: pinned in the solver, left out of profiles and passive
drawing, drawn in the external colour, never dragged, and projected again
once per editing session (in place when the curve is the same kind). File › Export
(`app/export.rs` over `kernel_ogeom::export`) writes the visible or the
selected bodies as STEP, or as STL or 3MF meshed afresh at the dialog's
tolerance and welded closed, on a thread of its own; the start page's
Export for printing walks it on the pocketed example. File › Send to
slicer writes the visible bodies to `$TMPDIR/printcad/slicer/` in the
format of `PrintingSettings::slicer_format` and runs
`slicer_command` on it (`{file}` places the path, else it goes last;
empty uses `xdg-open`). A save carries a
CPU-rendered preview (`thumbnail.rs`, in the save worker) as the
container's first entry, `thumbnail.png`, which
`Document::read_thumbnail` reads without unpacking the rest; the recent
cards show it. What's new reads `crates/app_shell/RELEASE_NOTES.md`, and
a test fails when the running version has no entry there. The Edit menu's Cut/Copy/Paste go to the
active bench as `MenuScope::EditMenu` commands (the sketcher keeps a
geometry clipboard); the view toolbar's clipping plane (`camera/section.rs`, per tab, a
plane square to X, Y or Z) reaches the renderer as
`FrameSubmission.clip_plane`: every scene shader and the pick pass write
a clip distance, the cut's back faces draw as a flat darker section, and
CPU edge picking skips what it hides; the toolbar's Measure arms a two-click distance
readout drawn over the scene (Escape puts it away); the print bed is a
line box from the Printing preferences. The design shows Part Design and Sketcher elements the app
does not implement yet. They stay on screen as disabled controls with a
`// PLANNED: <what it does when built>` comment next to them and a
`ToolDescriptor::planned(note)` on tools (`tool_button` renders them dim,
the host never dispatches a planned id). Nothing outside those two
workbenches gets a placeholder.

**Body placement.** A body has a `BodyPlacement` (`core_document/src/
placement.rs`, set by the `SetBodyPlacement` op). Its features, sketches,
datums and kernel shape stay in the body's own frame; the document keeps
the mesh the scene draws and picks placed (`imported_geometry`) and the
body's own beside it (`local_geometry`), so rendering, picking, edges,
bounds and previews need nothing. What crosses into the kernel is the
body's own frame: mesh-to-solid reads `local_geometry`, export passes the
placement (`ExportBody::transform`), a body boolean's tool carries its
placement relative to the target (`SolidOp::Boolean::tool_transform`).
Picks reach benches in world space; a bench storing a reference converts
it (`ctx.selected_face_in(body)`, `selected_edges_in`). The registry
places passive geometry and pick views by the feature's body; the
sketcher edits a sketch where its body sits and stores the plane back in
the body's frame. Every mesh face records its exact surface
(`TriMesh::face_surfaces`, `FaceSurface`), and a picked face carries it
(`FaceRef::surface`), so a picked bore brings its axis.

**Keyboard shortcuts.** One keymap (`ui/keymap.rs`) holds the
application's commands (`HOST`, ids like `file.save`) and every bench's
tools and registered actions, each with default keys
(`ToolDescriptor::shortcut`, `WorkbenchContext::register_action`,
`core_document::Chord`). The user's changes live in
`UserSettings.keyboard` by id and are edited in Preferences › Keyboard.
`take_pressed` runs at the start of each UI frame and removes the keys it
uses from egui's input: the active bench's keys win over the
application's, keys without Ctrl or Alt stay with a focused text field,
nothing fires while Preferences or the palette is open, and bare
number keys stay with a bench whose `takes_numeric_input` is true (the
sketcher while a length is typed). Shortcuts are single chords, never
sequences. Defaults: a plain letter picks a bench tool, Shift and a
letter its partner (a sketch constraint, a subtractive feature); 0 to 6
are the standard views, O/P the projection, Space and Delete act on the
tree selection (`HostState`). A tool key
activates the tool as a click would; an action key reaches the bench as
`WorkbenchInputEvent::Action`. Benches learn their keys in effect through
`Workbench::shortcuts_changed`. Contextual keys (Escape, Enter, Delete in
the sketcher, Escape for the measure tool) stay in the bench or host that
owns the moment.

**Commands and scripts.** `core_document::command` is the command
contract: `CommandSpec` (id, typed `ParamSpec`s, `extra_args`), JSON
`CommandArgs` in, `CommandResult` out, `spec.check` before every call. A
bench registers commands in `configure` (`register_command`, ids unique
across the registry) and runs them in `Workbench::run_command` with the
same context its tools get; the app's own are `doc.*` plus every keymap
command (`app/scripts.rs`). The `scripting` crate is Lua 5.4 (mlua,
vendored) and knows no command: a `scripting::Host` lists and runs them,
the prelude (`prelude.lua`) builds the `pc` namespace, `print`, `show`
and `help`. The console (`ui/console_view.rs`, output in the `console`
store) sends `UiCommand::RunConsole`, which queues it on the script thread
(`scripting::ScriptThread`, owning the engine; `PrintCadApp.script_thread`).
Each command the script calls comes back as `Event::Call` and runs on the
UI thread in `drive_scripts` (8 ms a frame, in the tab the run started in,
`in_script_tab`); `doc.rebuild` answers once the kernel is idle
(`script_rebuild`) rather than blocking a frame. The journal is held
(`OpJournal::hold`) from `Started` to `Finished`, so a run is one undo
step. The thread wakes the loop through `AppEvent::Script` and a busy
thread counts as async work. Stop sets the engine's stop flag.
Tools end in commands, which is what recording rests on: a bench calls
`ctx.record(id, args, result)` where a UI action ends in the code its
command runs (`HookOutcome.recorded`, carried through `PanelWriteback`
for panel hooks). The sketcher's click is `step::click`, shared with
`sketch.draw` (a shape is one call, flushed when the tool returns to rest);
drags are `step::drag`; Part Design records in `record_task` when a task
closes, diffing against what the command makes alone (`default_feature`);
Assembly in `record_joint`. The host maps its own UI commands in
`recorded_of` and keeps calls in `PrintCadApp.recording`
(`scripting::Recorder`, which names results and writes numbers exactly
so a replay meets single-precision values bit for bit); nothing records
while a script runs. The app's own
commands: `doc.*` (the pure ones in `scripts::document_command`, shared
with `headless.rs`), `app.*`, and every keymap command, the file ones
taking a `path` to skip their dialog. Script files: `script_library.rs`
reads `settings::scripts_dir()` every 2 s into `script.<name>` keymap
bindings (`Target::Script`), the Scripts menu, toolbar button and palette.
`printcad --script` (`headless.rs`) runs before the event loop, rebuilds
on the calling thread, logs to stderr. `docs/SCRIPTING.md`'s command
reference is generated; a test fails when it drifts
(`PRINTCAD_WRITE_DOCS=1` rewrites it).
Commands never open a task; Part Design's make features through
`create_feature`, the toolbar's own path, then merge named fields into the
feature's JSON. `kernel_ogeom/tests/scripted_part.rs` runs a script through
the real benches to a solid.

**AI agents.** The `agents` crate knows no command either: `rpc`
(newline-delimited JSON-RPC), `acp` (`AgentChat`, a worker thread per
agent process speaking the Agent Client Protocol: `ChatCommand` in,
`ChatEvent` out), `mcp` (the server core over a `ToolHost`) and `bridge`
(`printcad --mcp`, relaying stdio to the app's socket with a header naming
the chat). `app/mcp.rs` listens on `$XDG_RUNTIME_DIR/printcad/mcp-<pid>.sock`,
one thread per client handing each tool call to the UI thread
(`drive_agent_tools`); `call` and `lua` run as script-thread jobs
(`Job::Command`/`Job::Script`, `RunKind::Agent` answering the tool), so
tab pinning, one undo step per call and Stop come from the script path.
A change waits in `PrintCadApp.approvals` while its chat asks
(`asks_before_changes`); `CommandSpec::read_only` (declared by whoever
registers the command) is what never waits. `app/chats.rs` keeps
`PrintCadApp.chats`, each started with the relay as its MCP server
(every tool `always_load`, sent as `_meta."anthropic/alwaysLoad"`, so a
client that defers tools behind a search has them in its first turn;
`read_only` becomes `readOnlyHint`);
`ui/assistant.rs` draws them and answers with `UiCommand`s. The agent's
session options (`acp::SessionOption`: `configOptions`, or the older
`modes`/`models`) draw as the bar under the input; a change shows at once
and is put back if the agent refuses, an answer an agent-side update has
overtaken is dropped, and the choice is kept per agent in
`AgentSettings.choices`, put to each new chat in the agent's own order
(`put_choices`). A prompt's attachments (`acp::Attachment`, held on the
`Chat` until sent) become content blocks in the chat thread
(`prompt_blocks`, by the agent's `promptCapabilities`): pictures as
images, small UTF-8 files embedded, the rest as `resource_link`s. They
come from the "+" menu (`FileDialogKind::Attach`, `attach_view` over
`view_png`), a paste of file paths, or a drop on the panel (winit delivers
drops on X11 only). Agents are configured in `UserSettings.ai`. `docs/AI.md` is the user guide.

**Variables and formulas.** Any number a bench lists
(`Workbench::parameters`: a `Parameter` with a stable key, a JSON pointer
into the feature's data, a `Dim`, a scale for radians, an integer flag)
can be set by a formula. `core_document::expr` parses and evaluates them
(units checked by powers of length and angle, a bare number taking the
unit beside it or the field's, references `Object.property`, backticks for
names with spaces, rename rewriting in place). A feature's formulas live
in `FeatureNode::formulas` by key (op `SetFeatureFormula`, carried by
`AddFeature` so undoing a delete restores them); the bench's own data
keeps plain numbers, so no bench type changes. Variable sets
(`core.variables`) and the configurations table (`core.configurations`,
whose active row stands in for chosen variables' formulas) are body-less
feature nodes the registry presents itself. `evaluate::evaluate_document`
works out every slot once (loops, unknown or ambiguous names and wrong
kinds reported on the slot); `DocumentService::evaluate` runs it when
`mutation_seq` moved, lets each owner `settle` its evaluated data (a
sketch solves; reused while the unsettled input is the same) and applies
it as derived state: `Document::feature_values` is the data a bench
builds, draws and edits from, and a feature whose values moved is marked
dirty without marking the document edited. Results a bench records rather
than derives follow through `Workbench::values_moved` (the assembly
re-solves placements): the host calls it from `settle_formulas`, every
frame and in `close_gesture`, which every undo boundary goes through, so
an edit and what it moves are one step; headless runs settle after every
command. A value set by hand goes through
`DocumentService::set_parameter_value`. The UI: `ui_kit::widgets::
FormulaField` over `core_document::DocumentFormulas` (the document's last
values) in the property panel's Parameters group, Part Design's and the
Assembly's task fields and the sketcher's dimensions (bound ones drawn in
`SketchPalette::formula`); a selected variable set or the configurations
table shows its editor in the Data tab (`ui/variables_view.rs`), and the
tree's document row makes them. Commands: `var.*`, `config.*`,
`doc.parameters`, `doc.set_formula`, `doc.set_value`. `docs/VARIABLES.md`
is the guide.

**Tasks and undo.** A feature edit is a task in the right panel: edits apply
live, OK accepts, Cancel writes the opening snapshot back (or deletes the
feature the tool just created). `frame.rs` skips the per-frame
`journal.note` while a task is open, so one task is one undo entry;
`TaskClosed` closes the gesture.

Recompute loop: workbench edits document → features marked dirty via the
dependency DAG → `drive_part_recompute` (each frame) asks every bench for
its `rebuild_jobs` (a `BuildPlan` of `SolidOp`s per body, the bench settling
the dirty flags of the plan's features and inputs itself) → kernel worker
thread → results land in the document's imported-geometry sidecar →
rendered/picked like any body. A history that changed shape goes through
`invalidate_body`; a history jump or Recompute All through `invalidate_all`.

Import performance: the per-solid work and each mesh's face pass go through
`ogeom_core::parallel::map_ordered` (order-preserving, so output is identical
at any thread count, which `tests/step_import.rs` asserts). Never nest two
`map_ordered` passes: `tess::Faces::{Wide, Inline}` says which level owns the
threads. Import meshes inline from the model already in memory; a deferred
pass would have to parse every snapshot back, which cost more than the
meshing. `crates/kernel_ogeom/examples/import_bench.rs` reports the phase
breakdown; measure with it before optimizing. STEP text is decoded lossily
(exporters emit Latin-1 in string literals).

Progress/cancel: the worker installs one `Watch` per job
(`kernel_ogeom::{Watch, watched, Canceller}`, re-exported so app code never
depends on `ogeom` directly). `kernel_ogeom::progress::context` announces our
own labels prefixed with `CONTEXT_PREFIX`; ogeom announces its own stages on
the same thread-local channel. The worker's sink files them into a shared
`Activity` slot (not a channel: `mpsc::Sender` is `Send` but not `Sync`), the
status bar reads it each frame, and `Canceller` backs the Cancel button. Our
op/face/solid loops call `progress::checkpoint()` themselves, since
`triangulate_face` has no checkpoints of its own. Stages announcing
`(done, total)` (kernel-side, or ours via `progress::stage_at` fed by a
shared monotone counter in the parallel import loop) draw as a determinate
bar instead of the spinner; a new context resets counts to unknown.
The reader's warnings never reach the terminal one by one: `ImportedModel.report`
carries them (by kind with counts, the full prose, untrimmed face ids, skipped
keywords). With `diagnostics.import_report` on (Preferences › General, off by
default) `app/import_report.rs` writes them to
`$TMPDIR/printcad/import-reports/<stem>-<stamp>.txt` (temp, so the system
clears them) and logs the path; off, one line gives the counts and names the
switch. That file is what goes to the kernel's maintainer.
**Shape health.** Every imported body is run through the kernel's checker as
it is read (`kernel_ogeom/src/health.rs`, a few ms a body) and carries a
`ShapeHealth` on its `ImportedGeometry`: broken findings (an algorithm
reading the shape answers wrongly) versus suspect (harmless). A broken body's
tree row, and every row above it, draws in the danger colour with the
findings as its tooltip, and the tree and viewport menus offer "Repair
shape". The repair is an op (`RequestBodyRepair`, a history barrier like an
import, since the unrepaired shape would have to be re-derived from the
file); the repaired geometry is derived from it by `drive_shape_repairs`
(`recompute.rs`), so a peer's request and a reopened document repair the
same way. The property panel's Physical group (volume, surface area,
centre of mass) is measured by the kernel worker on demand for the body the
panel shows, once per geometry revision (`drive_measurement`), never during
an import, where it would cost ~15 ms a body. **Announcement discipline:** `progress::
context` marks a *phase* and resets the display: call it once per phase,
never per body or per face. Anything emitted inside a loop is
`progress::detail` (a kernel-style sub-stage: shown under a sequential
phase, ignored while our counted `stage_at` loop owns the display) or
`stage_at`. Our counted stage speaks alone; the kernel's per-body stages
from twenty threads are noise, not information; the 1 s frame log's
`status_changes` counts status-text changes per second (a readable bar
changes ~1/s; a slot overwritten by threads changes every frame).

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
4. When the fix lands: bump the ogeom rev pin in the workspace `Cargo.toml`,
   un-ignore the matching tests, rerun the full suite.

## Invariants: violate these and things break subtly

- **`app/gfx.rs` field order IS the teardown contract** (struct fields drop in
  *declaration* order): renderer before window. Do not reorder. Inside the
  renderer the same rule bites: anything that frees device objects in its own
  `Drop` (the egui renderer) must be `take()`n and dropped in
  `RendererCore::drop` BEFORE `destroy_device`, or it runs on a dead device:
  a hang or segfault at exit plus a wall of "leaked objects".
- **Vulkan validation layers default to debug builds only**
  (`RenderSettings::default`); `PRINTCAD_VULKAN_VALIDATION=1` enables them for
  a release run. `PRINTCAD_EXIT_AFTER_MS` quits through the real exit path
  after a delay (keeps the loop awake) so teardown can be timed on a loaded
  document.
- **UiLayer must never own state the host mutates.** `active_tool` and
  `active_workbench` are seeded from `UiFrameInputs` every frame. A parallel
  copy in the UI caused an infinite New-Body loop once. A bench talks back
  to the host only through `ctx.request(HostRequest)` (tool, selection,
  journal label, bench switch or start-on, camera, finish editing) and
  `ctx.active_document_object`; `HookOutcome::take` collects them and
  `apply_hook_outcome` applies every one, from every hook site (a panel
  hook's arrive as `UiCommand::HostRequest`; a lifecycle hook's switch
  requests are the one thing dropped, since it runs inside a switch).
- **Adding a UI action** = one variant in `ui/commands.rs` + one arm in
  `app/commands.rs::apply_ui_commands` (two-phase dispatch preserves ordering).
- **Every user-edit mutator on `Document` records exactly one op; derived
  state never does.** Mutators follow validate → resolve (ids, timestamps,
  seq) → build `DocumentOp` → `apply_op` → record; replay runs the same code
  live edits ran (`core_document/src/op.rs`, tests in `tests/op_replay.rs`).
  Dirty flags, recompute errors and imported-geometry sidecars are per-replica
  consequences, excluded from the replicated projection. The outbox is
  `#[serde(skip)]` and Clone-EMPTIES: snapshots carry state, never pending
  ops. Never add a `&mut` escape hatch to `Document`; capture is only total
  because none exists.
- **The app is a client of a document server, one connection per tab**
  (`core_document/src/server.rs` trait = the wire protocol; `crates/doc_server`
  has the `printcad-serverd` daemon (one per document, unix socket under
  `$XDG_RUNTIME_DIR/printcad`, single client, exits on disconnect) plus the
  `DirectFiles` fallback). The daemon stores opaque `.prtcad` bytes and op
  envelopes (`<file>.oplog.jsonl`), never deserializing a `Document`. Ops
  recorded before a document has a file go to `unhomed-<socket hash>.oplog.jsonl`,
  keyed by the socket, and an untitled tab's socket carries the tab's id
  (`socket_path_for_untitled(tab)`), so two unsaved documents never share
  a log; one file for all of them meant the first to save took the other's
  history. Saves cross as client-serialized bytes with `at_seq`;
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
  `#[serde(skip)]` sidecars (save snapshots depend on it; `undo.rs` tests
  pin it). Solids stay derived: `after_history_jump` re-marks part features
  dirty.
- Kernel shapes are plain `Send + Sync` data; tests run in parallel with no
  serialization mutex. The kernel-worker thread exists for UI responsiveness,
  not safety.
- **Sketch endpoint snapping REUSES point ids**: that shared-vertex topology
  is what makes profiles closed for `profile::extract_wires`. Don't create
  coincident duplicate points.
- **Pocket/Groove cut AGAINST the sketch normal by default** (a face
  sketch's normal points out of the material, so the default digs in).
- **NDC is Y-down**: the camera bakes the Vulkan Y flip into `view_proj`.
  Transform helpers live in `core_document::runtime` (ctx methods + free
  functions); mirror them, never re-derive with a different convention.
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
document open/save, file dialog, deferred import), a 6-DoF puck is deflected,
the camera tween or bench orbit is running, or egui asked for a repaint (`repaint_delay` == 0; finite delays
become `WaitUntil`, e.g. caret blink). Otherwise the event loop sleeps in
`ControlFlow::Wait` until the next OS event. Consequences: anything that
completes on a background channel must be covered by one of the
"work pending" flags or it will not surface until the next input event; a
thread whose work produces no window event (the 6-DoF reader) must also wake
the loop itself, through `EventLoopProxy::send_event` and the `AppEvent`
user event; and
`fps_cap` now caps the *active* rate rather than implying continuous
rendering. `PRINTCAD_OPEN_FILE` (several paths `;`-separated open a tab
each, or all into one scene with `PRINTCAD_OPEN_SAME_TAB`) / `PRINTCAD_OPEN_DOC` /
`PRINTCAD_BENCH_ORBIT` / `PRINTCAD_EDGE_MIN_PX` / `PRINTCAD_NO_EDGES` are
bench hooks (frame.rs, mesh.rs); `PRINTCAD_BENCH_SKETCH=1` opens a
constrained sketch for editing and `=pad` pads it and opens the Pad task;
`PRINTCAD_BENCH_SELECT=<n or name>` selects a body once it has geometry,
the way a click on its tree row would, so a capture shows the selection
overlay; `PRINTCAD_BENCH_CLICK=<fx>,<fy>` snaps to a corner view and makes
one selection click at that fraction of the viewport, logging what the
pick, the edge test and the face hover saw and what got selected, and with
`PRINTCAD_BENCH_TOOL=<tool id>` then runs that tool on the selection as a
toolbar click would and logs every feature's rebuild error (frame.rs);
`PRINTCAD_BENCH_REPAIR=1` asks for the repair of every broken body once,
`PRINTCAD_BENCH_CONVERT=1` the conversion of every mesh body. Any of these skips the start page. The 1 s `printcad.frame` log reports
fps + phase costs while frames are being produced.

Face-boundary edges draw on every frame, moving or still. They are cheap
next to the solids: a 123-body assembly (9 M triangle indices, 419 k edge
indices) spends 1–2 ms of its frame on the edge pass, and a 272-body one
(133 M triangle indices, ~130 ms/frame) spends none, because
`PRINTCAD_EDGE_MIN_PX` (24 px of body AABB on screen, mesh.rs) has already
culled every body's edges. Measure with `PRINTCAD_NO_EDGES=1` against the
same orbit before assuming the pass costs anything.

**The 3D scene is cached between changes.** The scene pass resolves into a
persistent scene image and runs only when `scene_fingerprint(frame)`
(`render_vk/src/core.rs`) changes; every frame copies that image under the
UI pass. UI-only frames (hover, panels, typing) therefore cost ~2 ms on any
model. **Completeness of the fingerprint is the contract**: anything the
scene pass reads (camera, viewport, lighting, per-body id/revision/mesh
pointer/color/highlight/wireframe) must be hashed there,
or a change shows stale. The status bar shows both numbers because they are
two things: `FPS` (UI frames presented) and `scene: N/s` (scene redraws;
"cached" when zero). `PRINTCAD_BENCH_SPIN` keeps the loop awake with no
scene change (expect zero redraws); `PRINTCAD_BENCH_ORBIT` expects one per
frame.

## Interaction model (current bindings)

Tabs: one document per tab (`ui/tab_bar.rs`, under the menu bar), Ctrl+T
new, Ctrl+W close, Ctrl+Tab / Ctrl+Shift+Tab cycle, middle click closes;
a fresh tab shows the start page and New/Open take it over, a tab with
content keeps its edits and the new document opens beside it; closing
asks about unsaved edits per tab, quitting asks for each dirty tab.
MMB drag = orbit (MMB click = pivot pick) · RMB drag = pan · RMB click on a
body = context menu (`ui/context_menu.rs`: show in tree, select body, hide,
then whatever the benches offer through `menu_items` for
`MenuScope::ViewportBody`; the host owns the open `ViewportMenu`, the UI
draws it and answers with commands; tree rows and the start page's New
cards take bench entries the same way, and a pick runs the bench's
`on_command`) · wheel = zoom · LMB = select (click sketch → tree-select; click
solid → face-first, double click → the whole body the face belongs to, one
part of an assembly, and in Part Design the tree opens to its row; LMB drag
in sketch = box select; ctrl = additive). The
face under the cursor draws translucent in the hover paint (an edge within
reach takes the hover instead, as a line), resolved from the pick's point
and kept while that point stays on the same face; the
selected face, or the whole selected body, draws as a translucent overlay
over itself, never as a tint (`rendering.selection_color` /
`selection_opacity`, Preferences › Display › Rendering) through the
renderer's blended pass: any `BodySubmission` with `opacity < 1` draws
after the opaque bodies and their edges, depth-tested, never writing depth,
and the pick pass skips it: an overlay is never what the cursor is over.
While editing a sketch the view is locked planar (orbit + cube rotation
disabled; pan/zoom/roll allowed). In the tree, bodies and features start
open and imported assemblies start closed; the filter looks through closed
branches, a double click or "Show in tree" opens a body's way to itself,
and a body row's double click or "Select body" selects the whole body.
Every row that stands for something drawn has an eye: features, imported
parts and bodies (`Body.hidden`, `SetBodyVisible`, undoable), and the
viewport's Hide hides whatever body was clicked; a hidden body is out of
drawing, picking and the clip bounds (`imported_body_effective_visible`). An
instance row that wraps a single part is one row with the instance's id and
the part's body: resolve it through `Document::body_of_imported_object`,
never through the node's own `body_id`, which an instance lacks. The window
opens
on the start page (`Screen::Start`); the recent list lives in
`settings::recent`.

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
  icons, motion drawings and fonts are vendored from it.

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
  PR or commit references in comments; the one sanctioned place for a
  kernel issue number is the `#[ignore = "kernel: … (ogeom-rs#N)"]` string.
  `lint-comment-rot: ignore` on a line opts it out when a reference is
  load-bearing. The tracked pre-commit hook runs it in `--staged` mode over
  the lines a commit adds; enable once per clone with
  `git config core.hooksPath .githooks`.

## Known approximations / roadmap

- `TriMesh.faces` names the kernel face each triangle was cut from, so a click
  selects a whole face, a bore or a fillet as much as a flat side
  (`app/input.rs::face_submesh`). A mesh without faces (a sketch, a datum, a
  document saved before they were recorded) falls back to the plane through
  the hit. Persisted references are still geometric: dress-up edge selection
  and up-to-face terminations carry a sample point + normal (`FacePick`),
  re-resolved against the current solid each rebuild, because a rebuilt solid
  numbers its faces afresh. `TriMesh.edge_ids` names the kernel edge of every
  outline segment (the tessellator draws the outline from the kernel's own
  edges, each to the chord the faces agreed on; triangle boundaries are only
  the fallback), so a click within a few pixels of an outline picks the
  whole edge (`app/edges.rs`: hover, Ctrl-additive selection, highlight line
  bodies, the hover card's edge length, the measure tool's snap). Benches
  see picked edges as `ctx.selected_edges` (point, direction, length);
  Part Design's fillet and chamfer store them as `EdgeSel::Edges` probe
  points the kernel resolves through `EdgeSelection::Near`.
- "Through all" derives its length from the base solid's bounding box; up-to-
  face trims with a half-space, so only PLANAR target faces terminate exactly
  (curved to-first/to-last faces stop at the profile-centroid hit distance).
- Helix with height 0 (flat spiral) is rejected; use a small pitch instead.
- Hole threads are standards data only (tap-drill / ISO 273 clearance
  diameters); no helical thread geometry is generated.
- `orientation_cube/mod.rs` (1340 LOC) still needs the camera-style split.
