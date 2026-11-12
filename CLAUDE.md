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
```

- The document server is a separate binary the app spawns from its own
  directory, so a release run needs `cargo build --release` for the whole
  workspace: `-p app_shell` alone leaves `target/release/printcad-serverd`
  missing and the app falls back to direct file I/O with a warning.
- No system CAD libraries needed: the ogeom kernel is pure Rust. The
  workspace `Cargo.toml` pins it by `rev` on its repository's main; a new
  kernel fix is taken by editing that `rev` (the kernel's thread sends the
  commit, issue and final API after each merge). Only `kernel_ogeom`
  depends on it, and nothing published to crates.io does. A commented
  line in `[patch.crates-io]` points at a local checkout for kernel dev.
- 6-DoF input (SpaceMouse and the like) comes from the `sixdof` crate
  (crates.io, this project's own), consumed by version exactly as the kernel
  is, with the same commented `[patch.crates-io]` for local work. It needs no
  system library: it speaks the spacenavd socket
  protocol itself, with the display-server (Magellan) protocol behind its
  `magellan` feature, and on Windows and macOS (without spacenavd) reads the
  device's USB HID reports, giving the numbers the daemon would. Nothing is required to build or run without a device.
- Workbench packages build in `sdk/` (`cargo build --release --target
  wasm32-wasip2`, the target named in `rust-toolchain.toml`);
  `cargo test -p wb_wasm` builds them itself. At start `app/packages.rs`
  loads what `settings::workbenches_dir()` holds as
  `UserSettings.packages` allows (Preferences › Workbench packages
  installs from a file or a GitHub address, updates, removes, turns off
  and grants, all while the app runs; its Browse tab is the workbench
  store: `wb_wasm::store` reads registries' indexes, each a store the user
  keeps (`UserSettings.packages.stores`, a new install starting with the
  registry repository `PrintCAD-wb-repo`'s, removable like any;
  `app/packages.rs::StoreView`, one `StoreState` per address, each kept
  for offline in `settings::store_cache(url)`, fetched as
  `PackageNews::Store`, quietly at start with the update check and again
  when the list changes), and `install_listed` installs a listing's
  release checked against the index's sha256, recording its repository so
  it updates as any GitHub install; the page opens on a third-party
  warning, lists every store's packages (a store filter, each card naming
  its store when there are several), and the Stores tab beside it adds and
  removes stores;
  installed packages a store lists show Listed, and one taken off is
  warned of once a run; a package may declare kinds bought
  (`Registration::bought_kinds`: the parts list marks their bodies bought,
  and exports leave them out, until the user clears it); `PRINTCAD_STORE_INDEX` (`;`-separated) stands in
  for the list; `PRINTCAD_NO_UPDATE_CHECKS` leaves out the looks a start
  makes on its own, packages' updates, the stores' lists and the app's
  latest release, so many runs at once stay within GitHub's limits). Network work and compiling
  (installs, update checks at start, updates, turning one on) runs on
  threads reporting through `app/packages.rs::PackageNews`, drained each
  frame; the registry then changes on the UI thread: `unload_bench` moves
  every tab off the bench, drops the editing state tabs kept for it and
  keeps its settings, then `DocumentService::unregister_workbench`;
  `load_bench` registers the new one (`workbenches::register_prepared`)
  and `invalidate_all`s its kinds in every tab, so its features rebuild
  with the version running. A feature carries `made_by` (the package and version that
  last wrote it, restamped by `SetFeatureOrigin` whenever a package
  writes its data) and `package_source` (its GitHub repository, when
  installed from there); one whose kind no bench claims shows "Needs …"
  in the tree, keeps its data, cannot be deleted, and its tree menu offers
  to install the package from `package_source`; opening a document warns
  of each package it needs and does not have running
  (`report_missing_packages`).
- The executable is `printcad` (package `app_shell`). A release is the
  app's version in `crates/app_shell/Cargo.toml`, its section in
  `RELEASE_NOTES.md`, its designs in the corpus
  (`PRINTCAD_WRITE_CORPUS=1 cargo test -p app_shell every_corpus` saves
  each recipe's part into `crates/app_shell/tests/corpus/<version>/`
  with its measures, which every later build must rebuild to; the
  numbers are never changed to follow a newer build), and a tag `vX.Y.Z`: `.github/workflows/release.yml`
  builds Linux (Ubuntu 22.04, `.tar.gz`), Windows (`.zip`) and macOS (one
  universal `.app` in a `.dmg`, ad-hoc signed) through
  `scripts/package-release.sh` and publishes them as a GitHub release.
  The Linux archive carries `install.sh` (`scripts/linux-install.sh`: a
  per-user install with its menu entry and icon). The application icon is
  drawn in `crates/app_shell/assets/icon/` (`printcad.svg`, and
  `printcad-small.svg` for 48 px and below); `scripts/app-icon.sh` renders
  the PNG, `.ico` and `.icns` the builds carry: the window icon
  (`window_icon`), the Windows program's resource (`build.rs`,
  `winresource`), the macOS bundle's, and the desktop entry's, which the
  window finds through its app id `printcad`.
  The CI's `platforms` job runs clippy, and the tests when a change can
  differ by platform, on Windows and macOS (Testing conventions).
- The website (GitHub Pages) is `site/` (a hand-written landing page,
  `index.html`, `style.css` and `main.js` over a small WebGL2 viewer,
  `gl.js`, drawing the parts in `site/assets/models.bin`, which
  `site/tools/pack-models.py` packs from STL exports of the scripts in
  `site/tools/scenes/`; the guides' stylesheet `guides.css` and index
  `site/guides.md`) plus every guide in `docs/` turned into a page by
  `scripts/build-site.sh` (pandoc, `site/tools/`: the page template and a
  filter sending guide links to pages and source links to GitHub);
  `.github/workflows/pages.yml` builds and publishes it on pushes touching
  them. A new guide goes into the script's `guides` list and
  `site/guides.md`.
- STEP tests use the bundled fixture
  `crates/kernel_ogeom/tests/data/box_native.step`; set
  `PRINTCAD_TEST_STEP_FILE` to test against a richer model. (`box.step`, from
  another exporter, writes its edges as SURFACE_CURVE wrappers, which
  `imports_a_step_file_with_surface_curves` reads.)
- `[profile.dev.package."*"] opt-level = 3` in the workspace `Cargo.toml` is
  load-bearing, not tidiness: the kernel is numeric code and runs ~26x slower
  unoptimized, which made a large STEP import look like a hang. Our own crates
  stay unoptimized (fast rebuilds, readable backtraces), so a debug build is
  still ~1.4x slower than release, so use `--release` when timing anything.
- `crates/kernel_ogeom/examples/import_bench.rs` prints the phase breakdown of
  an import; reference timings live in the import-performance memory.
- wgpu's validation errors are routed into `tracing` (target
  `printcad.gpu`), with the graphics API's own validation under them where
  it is installed. Keep the app validation-clean.

## Crate map / dataflow

- `bench_api`: what a workbench package and the app exchange (serde,
  builds for wasm32-wasip2): manifest, registration, nodes, rebuild plans,
  input and pointer, the `Frame` a bench draws, declared panel `Widget`s
  and `PanelEvent`s, menus, requests, the `calls` a package may make. The
  WIT world is `bench_api/wit/workbench.wit`; its values cross as JSON of
  these types, bulk data as lists. Published on crates.io with
  `kernel_api` and the SDK as `printcad-bench-api`, `printcad-kernel-api`
  (library names unchanged, dependents use the `[workspace.dependencies]`
  entries) and `printcad-bench-sdk`, versioned by the contract (0.1.x is
  `printcad:workbench@0.1`) and released by a tag `sdk-vX.Y.Z`
  (`.github/workflows/sdk-publish.yml`). A patch release only adds
  (`docs/PLUGINS.md` › Versions): the enums the host sends are
  `#[non_exhaustive]` unless the `exhaustive` feature, which the workspace
  turns on, is; package-filled structs derive `Default`; a variant a
  package builds (`Widget`, `Request`, `SolidOp` and what it holds) gains
  no field before 0.2. The SDK keeps its own `wit/` copy, which a
  `wb_wasm` test holds equal to `bench_api`'s. Panel blocks (0.1.4:
  `Widget::Section`, `Item`, `Items`, `Filter`, `Status`, `Stack`,
  `Flow`, `Fill`, `Fields`, `Inline`, `PickChip`, `Entry`, `TaskHeader`
  (lifted into the task header by `wb_wasm`), `Quantity`, `Check`,
  `Select`, `Grid`, `Alert`, `Card`, `Caption`, `Gap`, `Plot`,
  `Swatches`, `Color`, `Range`, `RowButton`, `Selectable`, `Toggles` (0.1.8: icon buttons in a row, each lit while on, as an editor shows alignment), `Gallery`
  (pictures kept by key), `Settings` (a Preferences-style group), each
  over a `Default` struct, and the `PanelEvent`s `Hover`, `HoverPart`,
  `Activate`, `ContextMenu`, `Tag`, `Badge`, `ItemText`, `Selection`,
  `Filter`, `Clear`, `Color`) are drawn by `bench_panel` through the same
  `ui_kit` widgets the native panels use, every section's fields on one
  `ui_kit::widgets::LabelColumn` (a `Grid`'s on egui's own two-column
  grid);
  the SDK's `panel` module builds them. `bench_api::panel::Form` is the
  one walk every bench's declared panel goes through: without an event it
  lists the widgets, with one the binder whose widget it names
  (`number`, `check`, `choose`, `index`, `text`, `button`, `action`, `toggles`,
  each naming its widget through `panel::Ids`) applies it to the value
  it binds and returns the widget unplaced (`Bound`); Design's `form.rs`,
  Surface's `panel.rs` and Inspection's `tolerance_task.rs` wrap it with
  their own look, and the Assembly's `panel/` and the app's own tasks
  walk it directly. A frame's `card` (0.1.7:
  `FloatingCard`, `PanelSlot::Card`, `PanelEvent::CardClose`) is a card
  over the view beside what it edits. A manifest's `min_contract`
  (`bench_api::contract_satisfies` against `CONTRACT_VERSION`, the
  crate's own version) refuses a package needing a later patch.
- `workbenches/wb_wasm`: workbench packages (`docs/PLUGINS.md`, RFC 0001).
  One wasmtime engine (`engine.rs`: an epoch ticker that runs only while a
  call or job does, 25 ms for frame and input calls, 1 s otherwise; the
  compiled component cached beside `bench.wasm`), `exports.rs` (the
  guest's exports as the `Exports` trait, whichever runtime holds the
  instance, `Budget`, `Fault`), `host.rs` (`Reach`, the host's side of an
  instance: what the call in progress may reach, `Access::None/Read/Write`,
  a raw pointer valid for that one synchronous call; the `doc.*` calls a
  package makes on its own kinds only, recorded like any edit),
  `geometry.rs` (the read-only `geom.*` calls, the kernel asked in the
  body's frame and its answer named from the body's mesh by where it
  lies; a plan's `CosmeticThread`s drawn as `passive_geometry` and,
  those with a `designation`, answered as `cosmetic_threads` from their
  face (`thread_face`), the plan asked again only when the body's history
  changed), `guest.rs`
  (wasmtime's bindings and store; instantiate with WASI, the package's
  `data/` preopened as `/data`, the network only when granted, a memory
  cap; a trap or overrun replaces the instance, three turn the bench off),
  `jobs.rs` (a job in its own
  instance on its own thread, progress and cancel, native helpers under
  the `helper` grant; each told only in the tab it started in, by the
  `Session` that tab keeps of the bench, and while it holds the same
  document, else stopped and dropped), `bench.rs` (`WasmWorkbench`: the `Workbench` trait
  over the guest, frame cached by document seq, selection and events,
  world-space drawing projected by the host every frame, icons and short
  labels interned), `package.rs` (`bench.toml`, `.pcbench` tar archives,
  install keeping `data/`, uninstall, pack), `remote.rs` (GitHub releases:
parse a repository or release address, the release's `.pcbench` asset,
download checked against GitHub's sha256 digest, `source.json` beside the
package, `check`/`update` refusing a different package id; network behind
the `Fetch` trait so tests stand in their own). `sdk/` is a workspace of its
  own for wasm32-wasip2 (excluded from the root one): the guest SDK
  (`Bench` trait, `host` calls, `bench!`), `examples/gear`,
  `examples/peg` (a tool's gesture: a drag previews, a floating card
  holds the draft, OK writes it as one step, anything else drops it),
  `tests/rogue` (misbehaves on request) and `tests/probe` (a sketch's
  profile, a cancelled job, a table cell, a formula, the save dialog:
  what the gear leaves out); `wb_wasm/tests/packages.rs`
  builds them with cargo and runs them through the host, the peg through
  `wb_wasm::testing::Harness` (a package driven without a window under a
  fixed view, tabs, card OK/Cancel and unload, the undo history kept by
  the app's rules; `docs/PLUGINS.md` › Testing a package). A card event
  for a card the frame does not show is not delivered, and a fault's
  restart (`Guest::restarts`) asks the frame afresh.
- `kernel_api`: pure data contract (TriMesh with per-triangle kernel face ids, ProfileWire w/ ellipse+B-spline
  segments, `SolidOp` = sweep/loft/pipe/primitive/dress-up/transform/boolean,
  ExtrudeTermination, TessellationSettings, ChainError, `PointMatch`: the
  key a mesh point hashes by when welding or matching corners, exact or a
  grid). No geometry code but `arc`: which three points make a profile
  arc (the kernel's collinearity ratio, `COLLINEAR_SINE`, and a radius
  limit, `MAX_RADIUS`, past which the kernel's circle misses its points)
  and where it runs, the one rule the sketcher's regions and drawing
  tools, Design's borrow and the kernel adapter's profile hold arcs to.
  `JOIN_MM` (1e-3) is how near two curve ends are one point across the
  contract: a profile wire's segments, the kernel's section runs, and the
  ends Section to sketch, borrow and the sketcher's model picks read off
  a solid's edges. The sketcher's own join, `curves::END_SLACK` (1e-4:
  regions, projected ends, DXF), is held no wider at compile time, so
  what a sketch closes the kernel builds.
  Every weld in `kernel_ogeom` goes through
  `mesh_intake::intake` with a named match (`SAME_POINT` for a solid's
  faces, scans read and exports, so a mesh is closed in all or none;
  `HISTORY_POINT`, exact, for a mesh history's start); benches and
  `surface_texture` key by `PointMatch` with a constant of their own.
- `kernel_ogeom`: pure-Rust kernel adapter. STEP and IGES import share one
  path after the read (`import.rs`, reader chosen by extension, `is_iges`);
  IGES solids and closed surface groups become bodies, open sheets are left
  out with a log line. STL, OBJ, 3MF, PLY, glTF and VRML (`mesh.rs`; glTF and VRML are in
  metres and scaled) import as **mesh bodies**: triangles and no shape snapshot (`Document::is_mesh_body`),
  welded where normals agree within 30° and outlined at creases and holes;
  they draw, hide and pick and take no features. "Convert to solid" (tree
  and viewport menus) records `RequestMeshSolid`, a history barrier, and
  `drive_mesh_solids` derives the B-rep with the kernel's `solid_from_mesh`
  as the mesh is (coplanar triangles merged into faces, curved stretches
  left as facets with every vertex kept, `ShapeHealth::faceted`; a mesh
  that does not close becomes an open shell, said in the log), after which
  the body is an ordinary imported solid. "Refine shape" (`RequestBodyRefine`,
  a history barrier, offered by `Document::can_refine` while the body is
  faceted and has no base) has `drive_shape_refinements` run the kernel's
  `refine_solid` (`mesh::refine_blob`), which rebuilds the cylinders, cones,
  spheres and tori among the facets; a refine that fails or would not
  close leaves the facets and is not asked again that session. A faceted
  body's row carries a FACETED badge, and its conversion logs a warning,
  saying features on its curved areas may fail until it is refined. The files' annotations (`annotations.rs`: PMI callouts,
  undrawn semantic dimensions/tolerances/datums, datum targets, notes)
  leave as `ImportedModel.annotations`, one per body a callout describes,
  placed with that body's occurrence, text formatted (`Ø 35 ±0.2`); IGES
  levels and groups as `ImportedBody.layers`. The app keeps them on the
  import's nodes (`ImportedObjectNode::annotation`/`layers`, an
  `Annotations` group under the model's root) and `app/annotations.rs`
  draws them as screen-space lines and labels (`rendering.show_annotations`,
  View › Annotations), placed and hidden with their body. STEP import builds bodies from
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
  instancing. A pattern with a path step (Design's Path Pattern, or a
  Multi Transform's Path step) plans `SolidOp::Pattern`, whose steps the
  chain turns into transforms where the op stands
  (`ops/pattern_steps.rs`: a sketch's chain, or an edge of the running
  solid found by its `EdgeProbe`, walked by arc length, an aligned copy's
  turn carried along it), so the plan stays a pure function of the
  document; `KernelQueries::pattern_transforms` answers the same for the
  task's occurrence dots. A kernel-side gap gets a test marked
  `#[ignore = "kernel: …"]`; grep for `kernel:` in `tests/`
  before assuming a feature is wired wrong.
- `feature_walk`: `#[derive(Walk)]` for `core_document::walk::Walk`: each
  field of a feature type walked by the role its `#[walk(...)]` names (a
  number formulas set, a list of them, a setting, a function) or by its
  own type, at the JSON pointer serde writes it to (it reads `untagged`,
  `flatten` and `rename`); a `FeatureId` is a feature read. Design's
  `DesignFeature` derives it: its `parameters` and `dependencies` are
  the walk's (`Walker::of`), and a field with no role fails to compile.
- `core_document`: Document (feature tree DAG, bodies w/ `tip`, tar `.prtcad`
  persistence), `Workbench` trait + runtime context,
  workbench registry (`service.rs`), core datums (`datum.rs`:
  plane/line/point/coordinate system + attachment + offset, shared across workbenches;
  an attachment on its own body's solid (a face, edge or point picked, a
  circle's centre, the centre of mass and axes of inertia) keeps its picks
  as `ShapeProbe`s, which Design's plan asks of the solid where the
  datum stands in the history (`BuildPlan::probes`, answered by
  `ChainRequest::probes`); the answers are derived state
  (`Document::store_probe_answers`) that `datum::derive` folds into the
  datum's values, so it and what is built on it follow the solid);
  `jobs`, the benches' work away from the window: `Keyed` (results by id
  and the key of their inputs, a newer key superseding a run still going,
  its late answer dropped: Mesh's and Design's deviation maps,
  Inspection's maps, volumes and tolerance readings, each in its bench's
  tab state) and `Ticket` (a `Job`, with progress and a stop flag, kept
  with the inputs it was asked under, its answer taken only while they
  hold: Inspection's best fit; Assembly's checks are plain `Job`s).
- `bench_panel`: the declared panels drawn with egui (`show`, `show_card`,
  `show_filtered`, `focus_entry`, `PanelOutput`, `FormulaSet`), and the
  formula fields over a document (`DocumentFormulas`,
  `formula_candidates`), on `core_document` and `ui_kit`. The benches take
  it behind their `egui` feature; app_shell always. `core_document` has no
  `ui_kit`: its own `egui` feature only gates the `Workbench` trait's
  drawing hooks (`ui_task_panel`, `ui_settings`, `ui_left_panel`,
  `ui_viewport_strip`).
- `core_document::renamed`: ids that were renamed (`wb.part` is
  `wb.design`, `part.*` commands and tools are `design.*`). `WorkbenchId`
  deserializes old ids as new, so documents and op logs read either;
  `DocumentService::command` finds a command by its old name, and the
  app's call intakes (script calls, agent checks, headless) turn old names
  into new before looking further; `UserSettings::rename_ids` moves bench
  settings, key bindings and toolbar groups at load.
- `doc_server`: the document server: `printcad-serverd` binary +
  `DaemonClient`/`DirectFiles` implementations of the `DocumentServer` trait;
  length-prefixed JSON frames with the container bytes beside them, never
  inside (`framing.rs`, `server::Payload`), since a document as JSON numbers is
  four times its size and overran the frame cap outright, integration-tested against the
  real spawned daemon (`tests/daemon.rs`).
- `doc_render`: what the app draws and lays out from a document with no
  window, linking neither the GPU, the window nor egui, so its tests and
  the save worker run without them: `thumbnail` (the CPU renderer: a
  save's preview, an animation's frames; `preview_shape`, a body in the
  colours it shows in), `proof` (the `view` tool's and `doc.picture`'s
  pictures, in the colours of an `Ink` the app fills from
  `ui_kit::tokens`, their labels in the bundled Plex face read from
  `ui_kit/fonts/`), `pdf` (the report writer), `face_pick` (faces picked
  on a mesh; the GPU pick's `document_triangle` stays in
  `app/face_pick.rs`) and `print_layout` (the packing and the bed's
  lines; the task's shown layout stays in the app). The headless
  commands (`app/scripts/document.rs`, `headless.rs`) are still in
  app_shell.
- `local_ipc`: what the app needs of the system to talk to its own
  processes, on every platform: path-named local sockets (`Stream`,
  `Listener`: the standard UNIX sockets, on Windows its AF_UNIX ones; names
  stay short, since a socket path holds about a hundred bytes),
  `runtime_dir`, `open_with_system` (`xdg-open`, `open`, `explorer`),
  `background` (no console window for a helper on Windows), `find_program`
  (`PATHEXT` lookup, so npm's `.cmd` shims start) and `program_name`.
- `ui_kit`: the design system, below the workbenches so their panel code
  can use it (behind their `egui` feature): `tokens` (the palette and
  size constants, named after the design's variables), `theme`
  (`apply_theme`, bundled IBM Plex Sans/Mono under `fonts/`, fetched by
  `scripts/vendor-fonts.sh`; `sans/sans_medium/sans_semibold/mono` font
  helpers), `widgets` (Card, overline, badge, key chip, toggle, check row,
  note card, the button set, `tool_button`, `section_header` and
  `section_fold` (every panel's folds: a hairline, the title in small
  capitals, a count and a summary while closed), `status_pill`,
  `filter_field`, `list_row` (`RowLook`: selected, problem, dim, and
  `related`/`lit` for rows tied to the item in hand) and `row_value` (a
  panel's list rows: icon, name, dim detail, value at the end;
  `list_row_with` draws its own detail), `chip`/`chips` (small rounded
  names inside a row, hoverable and clickable, "+N" past the room) and
  `badges` (glyph badges, capped with "+N"); the sketcher's lists use them
  over `wb_sketch/src/relations.rs`, `QtyField`,
  `select_field`, `PrefRow` + `pref_group`, `planned`,
  `completion::completing_text_edit` (a text edit whose dropdown offers
  the five names nearest the word at the cursor, arrows, Enter/Tab and
  clicks, fed by `bench_panel::formula_candidates` wherever a formula
  is typed), and `Tab` +
  `tab_plus`, the one tab every strip draws: documents, chats, the property
  panel's pages, Preferences), `icon` (the SVG
  set under `icons/`, vendored by `scripts/vendor-icons.mjs` into a
  generated `icon_table.rs`; `icon::texture/draw` rasterize with a
  font-free usvg, since the system-font scan is far too slow for 200 icons;
  `select.svg`, `expression.svg`, `chevrons-down.svg` and `home.svg` are
  hand-authored locals). The same
  table carries the `motion-*` drawings (200×200, their own colours, one
  per movement of a 6-DoF mouse), drawn through `icon::drawing`, which
  rasterizes for the size it is shown at rather than the icon size. A test
  fails when the table and the directory disagree. `markdown` draws the
  chat's messages: `pulldown-cmark` into a small block tree, each run of
  text one selectable label whose links are found from where a click lands
  in the laid-out text, code blocks with a copy button.
- `sketch_solver`: standalone std-only sketch mathematics: ordered typed
  `problem`, semantic `compile`, `curves`, `spline`, `residual`, LM `solve`,
  rank/DoF `freedom` and `diagnosis`, `soft` (measured points settling
  what the constraints leave free); optional serde. Its complete directory
  can be copied to another consumer without workspace inheritance.
- `workbenches/wb_sketch`: sketcher. Its model (`sketch.rs`: points,
  radii, constraint values, `SketchPlane`) is `f64`, stored as the solver
  answers it; `boundary.rs` is the one place values cross to and from the
  `f32` of screens, GPU meshes, picks and the package contract (incoming
  singles widened through their shortest decimal). `tools.rs` + `tools/{draw,modify,
  transform}.rs` (state machine), `geom2d.rs` (intersection/sampling math),
  `snap.rs` (`resolve`, the cue's and the click's snap: the sketch
  looked up once per query into a `Scene`, the kinds in a fixed order,
  ties settled by `Rank` from what the candidates are, never by ids,
  which a replay draws anew; `examples/snap_bench.rs` times it on a large
  sketch), `solver.rs` (sketch_solver adapter, storage write-back and
  application diagnostics; `locked_elements`, through
  `sketch_solver::freedom::locked`, the elements nothing left free can
  move, drawn in `SketchPalette::locked` until the whole sketch is fully
  constrained, cached per document edit),
  the line tool's drag from its end (`LinePress`, `step::line_arc_click`:
  the polyline's tangent arc, a `sketch.draw` point with `arc = true`),
  the line and polyline runtime (`chain.rs`: a click resolved once,
  `chain::resolve` after `step::land`, into the edit it proposes,
  `propose`, which the preview draws and the click applies, `apply`
  checking every point and curve it names before its first write; each
  segment closes its own undo step through `CloseToolGesture`, kept in
  `chain::Links`, so Backspace (`UndoToolGesture`) or the host's undo puts
  the gesture before it and an outside edit that takes its start ends it,
  `follow_history`),
  a scan's section (`scan.rs`: every visible scan cut by the sketch
  plane through `KernelQueries::mesh_sections`, kept by plane and scans
  in `scan_guides` and drawn dashed while editing; `sketch.fit_scan`
  fits each chain with `KernelQueries::fit_chain_2d`, the kernel's
  `fit_segments_2d`, and adds the pieces through the DXF path,
  `dxf::add_drawing`, so ends join; `drawing.rs` writes the
  sketch at true scale as layered R2000 DXF (`sheet`, through
  `KernelQueries::write_dxf_drawing`: elements, construction, picture
  edges thinned to 0.01 mm and text on layers of their own, mm or inch),
  SVG or CSV of elements and dimensions, each with a `traceability`
  line, `sketch.export_dxf`/`export_svg`/`export_csv`, Save as
  DXF/SVG/CSV; `candidates.rs` reads each size's and the relations
  drawn geometry nearly keeps, `sketch.candidates`, `sketch.design_values`;
  fitted pieces keep their slice points as `Sketch::soft`, which
  `solver::solve` hands `sketch_solver::soft::solve_soft`: the constraints
  met exactly, the points settling what they leave free),
  a picture's scale (`ImageScale`: given, nominal from the file's dpi,
  `images::dpi`, or calibrated by a distance or `sketch.calibrate_image`'s circle;
  an UNCALIBRATED label until then; `stretch` for a height scaled apart,
  `sketch.calibrate_axis`; scanner profiles in the configuration folder's
  `scanner_profiles.json`, applied on import by the file's dpi; `mirrored`,
  `sketch.align_image`, `sketch.turn_image`; `sketch.bolt_circle` through
  circles' centres),
  a picture's edges (`picture_edges.rs`: `image_edges` on a thread per
  asset, kept in pixels and drawn as guides; `sketch.fit_edge` grows a line or
  circle from the click along the edge while each point keeps to the shape;
  a point tool's click lands on the nearest edge at subpixel in `landing`
  (`edge_snap`, `nearest_on_edge`) unless `ctx.alt_down`, with a 4× loupe
  through its viewport's `images`),
  reference pictures (`images.rs`, `Sketch::images`, the file a document
  asset, decoded once each, shown through `ViewportOutput::images`
  as egui textured quads under the lines; `sketch.image`, `sketch.set_image`,
  File › Import for PNG and JPEG),
  the profile's regions shaded while editing (the viewport's `meshes` over
  `KernelQueries::profile_mesh`, cached by profile, a moving drag keeping its start's until release) and out of it (the
  passive geometry's `regions`, one `PassiveRegion` per region with its
  `FeaturePart`, the plain profile's regions stronger, which the host
  meshes once per revision and draws see-through behind the lines, lit
  in the hover or selection paint), loose ends ringed,
  a closed sketch's parts picked with no task open (`closed_parts.rs`:
  `Workbench::pick_part` answers the curve within reach or the region
  under the cursor, `part_lines` a picked curve's lines; the host keeps
  them as `DocumentSession::picked_parts` beside the selected feature,
  a click taking a region alone and Ctrl adding regions or curves, a
  region behind the face the pick found left alone, and hands them to
  benches as `ctx.selected_parts`; Design's Pad, Pocket, Revolution and
  Groove start with the picked regions, a revolution or groove about a
  picked straight curve, `region_pick::picked_regions`/`picked_axis`),
  `profile.rs` (closed-wire extraction), `regions.rs` (the planar arrangement:
  curves split where they cross, touch or branch, every bounded face a
  `Region` named by the elements round it and the side of each it lies on;
  `profile::extract_regions` outlines the ones named, which Design's Pad,
  Pocket, Revolution and Groove keep in `regions`, chosen by clicks in the
  view while their task is open, `wb_design/src/region_pick.rs`, or the
  task's checks; `sketch.regions` lists them), `overlay.rs` (screen-space rendering
  while editing), `glyphs.rs` (constraint icons and dimension layouts),
  `constrain.rs` (which constraint a toolbar action creates for the
  selection's shape; a tool the selection does not fit yet arms in
  `model_picks.rs`, taking sketch clicks and the host's edge picks as
  operands, projecting the edges as guides, reusing what the sketch
  holds, and recording `sketch.external` then `sketch.constrain`),
  `panel.rs` (the task panel, declared as blocks, `task_widgets`, drawn
  by `bench_panel` and answered in `task_event` through the
  sketcher's commands; `PanelState` keeps what each row and button stood
  for; a generated sketch's panel is `generator/panel.rs`, declared the
  same way), `dimension_task.rs` (a dimension's value, formula or new
  variable in a floating card by its label, `dimension_card`: opened by
  a new dimension, which waits in the card with the calls that make it
  until OK (`DimEdit::made`), a double click on a label or a value typed
  into its row; the draft worked out on a copy of the document through
  `evaluate_with_retained_parameters` and solved, shown through
  `displayed_sketch`; OK records the dimension, switches, variable,
  formula and value, and the host closes them as one undo step),
  `corner_task.rs` (a fillet or chamfer click on a corner opens a
  floating card by it, `corner_card`: the click replayed by `step::click`
  on the sketch as it found it at the size typed, shown through
  `displayed_sketch` and never stored until OK, which records it as the
  `sketch.draw` call; the host closes that as one undo step),
  `style.rs` (icons and
  names per element/constraint kind). `docs/SKETCHER.md` is the user guide.
- `image_edges`: standalone std-only subpixel edge chains in grey images
  (Gaussian, Sobel, ridges across the edge, thresholds from the image's
  noise with hysteresis, a parabola across the gradient, chains that know
  their brighter side); better than a tenth of a pixel on synthetic edges.
  `objects`: parts split from the ground by Otsu's level, measured (area,
  holes, centroid, calipers, long axis, outline); `sketch.image_objects`.
  Copyable like `sketch_solver`.
- `metrology`: standalone std-only measurement arithmetic for scan
  inspection (RFC 0004): `stats` (summary, percentiles, histogram, share
  within a tolerance; `NaN` is unmeasured), `form` (peak-to-peak,
  smallest and largest diameter), `orientation` (parallelism,
  perpendicularity, angularity, coaxiality, concentricity zones), `ramp`
  (jet and the public-domain perceptual ramps with end caps; where a
  value lands on a signed or banded scale) and `design` (the round value
  a measured size was designed at). `profile` (an outline's radius against angle, peaks by prominence;
  `sketch.image_profile`). `rigid` (Horn's closed-form fit of point pairs). `gdt` (a datum
  reference frame from up to three planes or axes, position, profile,
  runout, symmetry zones and the material bonus). `shrink` (a print's per-axis stretch from distances along normals,
  `inspect.compensation`). Copyable like `sketch_solver`.
- `workbenches/wb_mesh`: the Mesh bench (`docs/MESH.md`). A
  `MeshFeature` (kind `wb.mesh`, on the scan's body) holds one
  `kernel_api::MeshOp`; `Workbench::mesh_plans` lists each body's steps in
  order up to its tip, and the host's `app/mesh_histories.rs` builds a
  changed plan from the body's mesh start (`Document::mesh_start`, kept by
  the first step's `set_mesh_start`) on a thread through
  `KernelQueries::build_mesh_chain`, the result set as the body's
  geometry (derived), a failure on its feature's row; a start no plan
  uses is the body's mesh again. A marking is taken to the start's
  vertices by position (`marked_on_start`). Deviation of the edit
  (`mesh.deviation`, a toggle and a `ViewportOutput::scalar_maps` map measured on a thread)
  measures the mesh now to its start, a triangle whose middle lies off the
  start marking its corners added (grey). Commands `mesh.islands`,
  `mesh.delete`, `mesh.flip`, `mesh.cut`, `mesh.fill`, `mesh.smooth`,
  `mesh.offset`, `mesh.reduce`, `mesh.boolean`, `mesh.thicken`,
  `mesh.inspect`, `mesh.result` and `mesh.set`. `solids.rs`: Boolean
  (`MeshOp::Boolean`, its operand a `kernel_api::MeshOperand` snapshot
  taken when the step is made: a body's solid as native text with its
  placement, or the bounded solid a feature's owner offers through
  `Workbench::feature_solid`, kept per evaluation as
  `Document::feature_solid`; an Inspection element's primitive over its
  drawn extent) and Shell (`MeshOp::Shell`, hollow when closed, thickened
  when open); `mesh.trim_to_scan` cuts a solid body by the scan's break
  (`KernelQueries::trim_to_scan`, the kernel's `cut_by_mesh_region`) and
  lands the result through `replace_body_shape`, or on a new body beside
  a body built from features, which is hidden; `mesh.preview_cut` and
  `mesh.export` (`KernelQueries::write_mesh`) record nothing. Declared task controls edit each step live with OK/Cancel,
  listing triangle counts and the kernel's movement or verified reduction
  measurements. `Document::store_mesh_build` keeps these derived reports
  and source identities by the full plan; desktop and headless builds
  share it. A marking follows moved vertices through the source identities, a
  reduced vertex marked when every start vertex merged into it is
  (`Editable::simplify` keeps them).
- `workbenches/wb_inspect`: the Inspection bench (`docs/INSPECT.md`).
  An `Element` (kind `wb.inspect`, on the scan's body) is a shape fitted
  to the scan's marked points: `ctx.marking` (`MarkedSurface`, the
  host's marking of the tab, `app/marking.rs`) read against the body's
  `local_geometry`, asked of `KernelQueries::fit` (`FitRequest`,
  `FitAnswer`; `kernel_ogeom/src/fit.rs` over the kernel's least-squares
  fits, outliers past `sigma_cutoff` left out); the element keeps the
  marked runs and the answer, drawn as `passive_geometry` lines; a
  `method` (`kernel_api::FitMethod`: minimum zone, inscribed,
  circumscribed, through `KernelQueries::zone_fit` over the kernel's
  `fit::zone`) refits the points least squares kept, and every element
  keeps its points' minimum zone (`Fitted::zone`), the form its form
  tolerances read. With
  nothing marked a fit tool arms (`armed`) and a click on a scan grows
  the surface from the point under the cursor, or traces a circle or
  line along the sharp edge by it (`grow_at`, `KernelQueries::grow`, the
  kernel's region growing over the body's local mesh, its coincident
  vertices welded first since the importer splits them at creases;
  `inspect.grow`). A
  `Dimension` (`dimension.rs`, kind `wb.inspect.dimension`, dependent on
  its elements) reads a distance, angle or diameter off them each time
  it is asked, never stored, so a refit moves it; its nominal, plus and
  minus are `parameters`, and a band gives PASS/FAIL, drawn as labels
  while the bench is active (`spread` keeps them apart). A
  `Construction` (`construction.rs`, kind `wb.inspect.construction`) is
  a point, line, plane or circle made from other elements, likewise
  worked out on each read (`built_at`, into an `Element` dimensions read
  as a fitted one; a point is a sphere of radius zero inside the crate,
  answered to scripts as `Point`). Deleting an element takes what is
  made or measured from it. A `Comparison` (`compare.rs`, kind
  `wb.inspect.comparison`, on the scan) keeps a reference body and the
  map's settings; `maps.rs` measures its distances on a thread
  (`KernelQueries::signed_distance`, `kernel_ogeom/src/deviation.rs`: a
  solid meshed finely first, coincident vertices welded) keyed by the
  bodies' revisions and placements, and `ViewportOutput::scalar_maps` hands
  them to the host with a `MapLegend`; the reference draws see-through
  (`ViewportOutput::faded`); `hotspots.rs` groups the points out of tolerance into
  patches (labelled on the part, `inspect.hot_spots`) and `export.rs`
  writes the distances (`inspect.map_export`). `volume.rs` reads the scan against a solid
  reference as material on a grid of cells (`KernelQueries::volume_difference`,
  the kernel's `volume_difference`), missing and excess regions with volume
  and depth, labelled on the part while the comparison keeps a
  `volume_cell` (`inspect.volume_errors`, measured on a thread through
  `Maps::request_volumes`). An `ElementMap`
  (`element_map.rs`, kind `wb.inspect.element_map`) maps the scan
  against one element's ideal shape within its drawn extent, worked out
  in closed form (`inspect.element_map`). A `Pin` (`pins.rs`, kind
  `wb.inspect.pin`) keeps a point on the map shown on a scan
  (`shown_map`), its label the value there, and the cursor over a mapped
  scan reads it (`hovered_reading`; the overlay hooks' context carries
  the hovered body, point and face). A `Section` (`section.rs`, kind
  `wb.inspect.section`, `inspect.section`) keeps a plane in the scan's
  frame, an offset and a stack count; its cuts come from
  `KernelQueries::mesh_sections` (`kernel_ogeom/src/orient.rs` over the
  kernel's exact mesh section), worked out again whenever the scan or
  the section changes and kept by that key (`section::Cuts`), drawn as
  `passive_geometry` with the kernel the bench last saw. A `Shadow`
  (`shadow.rs`, kind `wb.inspect.shadow`, `inspect.shadow`) sits on a
  body of its own: `derive_on_geometry` asks `KernelQueries::mesh_shadow`
  (the kernel's mesh shadow and `fit_outline`, lines and arcs within a
  play band) whenever the scan, its placement or the settings change,
  keeping the outline in the document's frame as `made`, and the bench's
  `rebuild_jobs` extrude it into the body's block. `inspect.export_step`
  (Save elements as STEP) writes elements, sections and outlines as
  construction geometry, `construction_items` through
  `KernelQueries::write_construction_step`. `pmi.rs` answers
  `Workbench::semantic_pmi` for each Comparison's reference: tolerances,
  datums lettered in first-use order and dimensions, their face probes
  placed from the scan into world coordinates. A `WallThickness`
  (`thickness.rs`, kind `wb.inspect.thickness`, on any body; `inspect.walls`) is read by
  `KernelQueries::thickness` (rays or spheres, a solid exactly) through
  the same threaded `Maps::run`, thin painted hot. An `Alignment` (`align.rs`, kind
  `wb.inspect.alignment`) records how the scan was moved and the
  placement before and after; making one sets the scan's placement. Best
  fit runs `KernelQueries::register` (the kernel's point-to-plane ICP, a
  coarse search over principal-axis matchings then a refinement) on a
  thread, applied in `on_frame`. Datum, automatic and symmetry alignment
  (`inspect.align_datums`, `inspect.auto_align`, `inspect.use_symmetry`,
  `inspect.align_points` through `metrology::rigid`,
  `align_commands.rs`) use ordered elements or
  `KernelQueries::orient`/`symmetry_plane`. Commands `inspect.fit`,
  `inspect.grow`, `inspect.construct`, `inspect.dimension`,
  `inspect.tolerance` (`tolerance.rs`, kind `wb.inspect.tolerance`: form
  from an element's band, orientation through `metrology::orientation`
  against a datum element; position, profiles, runout and symmetry
  in `tolerance_read.rs` over `metrology::gdt`, a datum frame of up to
  three references, a profile measured against the comparison's
  reference through `KernelQueries::signed_distance` on a thread and
  kept by its inputs; double click opens `tolerance_task.rs`, its panel
  declared widgets that `walk` lists and applies a `PanelEvent` to through
  `bench_api::panel::Form`, drawn
  by `bench_panel` as a package's is), `inspect.compare`,
  `inspect.best_fit`, `inspect.move`, `inspect.reset_alignment` and
  `inspect.report` (`report_of`, as JSON, or CSV and text through
  `report.rs`; the Save report tool hands the CSV to `SaveFile`). Selecting a
  feature on a mesh body moves no tip (`move_in_time_to`).
- `viewport_camera`: camera pose, projection, rays, press-relative navigation,
  fit and timed transitions, shared with other applications. It depends only
  on `glam` and `serde`; its README describes the copy contract. The app's
  `camera/core.rs` converts preset-relative orientations, model coordinates
  and physical pixels; `camera/` owns input routing, clip planes and device input.
- `transform_gizmo`: move, turn and scale handles without a window
  (`gizmo::Gizmo`: layout, hit-test, press/drag/release and the delta from
  the press; `paint::Shape`s in colour roles, `paint::Ink`, for the host to
  draw in its palette; `hand` turns a delta into increments of the freedoms
  a handle stands for). Plain `glam` in `f64` and `emath` at its boundary,
  no printCAD type: it is shared with other applications, and a change goes
  to the shared copy first (its README).
- `surface_texture`: patterns pressed into chosen faces of a mesh for
  printing (`Texture`: pattern, projection, tile, depth, turn, inward,
  keep flat; `apply` welds, splits the chosen faces' triangles and their
  neighbours along shared edges so no crack opens, then moves each point
  along its normal, the rim fixed). The document keeps them on the body
  (`Body::textures`, `FaceTexture` with `FaceKey`s found by name like face
  colours, op `SetBodyTextures`, `doc.set_textures`); the app's
  `app/textures.rs` resolves faces and pictures (`Pressing`; a linked
  copy with none of its own takes its source's, `textures_of`, a mirrored
  copy pressing them into its mesh mirrored back and mirroring the
  result), makes the view's preview on threads (`drive_texture_previews`:
  in the body's own frame, drawn as a `MeshInstance`, one per
  `preview_key` so a source and its copies share it; the last one shown
  while a newer is made) and hands export a `finish` per body
  (`ExportBody::finish`, applied in the body's frame before placement);
  `ui/texture_task.rs` is the task. `docs/TEXTURES.md` is the guide.
- `workbenches` facade: `register_all_workbenches` (built-in benches, the
  listed ones then `ProjectImportBench`) and
  `register_packages` (installed packages, after them, as the user allowed;
  `PackageStatus` for the Preferences page).
- `project_archive`: a standalone reader of `.FCStd` archives (zip and
  `Document.xml` through `roxmltree`, the only XML parser: byte cap,
  allocation-hint count, a linear nesting scan before its recursive
  parse, `nodes_limit`), their objects, placements and sketch geometry,
  and the topology of a saved `.brp` shape (`saved_shape`: edges and
  faces numbered as features name them, with where each lies, read
  without the kernel); its `tests/data` holds the generated fixtures.
  `project_tools` turns one into sketches, Design features and bodies
  (`convert_into`, through the document's own mutators; each solid step
  a body starting with a Clone of the step before, the edges and faces a
  feature names found on the saved shapes and made picks, what cannot be
  rebuilt left out with what stands on it and listed as findings), and
  is registered by the
  `workbenches` facade as `ProjectImportBench`: no tools and no feature
  kinds, not in `REGISTERED_WORKBENCHES` (so no switcher shows it), only
  File › Import's FreeCAD file and `project.import`, which converts into
  a scratch document first so a refusal leaves the open one untouched.
  Its `project-tool` program (`inspect`, `convert`, `check`, which also
  holds each rebuilt solid to the file's saved shape by bounds and face
  count) is a developer tool, never published.
- `workbenches/fixtures` (`bench_fixtures`): ready-made scenes built from
  the benches' feature types (a dimensioned sketch, padded, pocketed) for
  the app's `PRINTCAD_BENCH_SKETCH` hook and tests; the host composes
  benches only through it, never by naming them. `scripts/check-workbench-seam.mjs`
  rejects concrete bench references in `app_shell/src` and direct bench
  dependencies; local gates and CI run the same checker.
- `workbenches/wb_design`: Pad/Pocket/Revolution/Groove/Loft/Pipe/Helix/
  Primitive/Hole/Fillet/Chamfer/Draft/Thickness/Offset solid/Scale/patterns/Boolean/Split/Piece features
  (`feature.rs`; every one that fuses or cuts carries `refine`, which the
  build follows with a `SolidOp::Refine` merging coplanar faces; the
  Preferences switch is only the default for new features, so geometry
  never depends on who rebuilds it; and an optional `boolean_tolerance`,
  the Advanced group's, which wraps each of the feature's ops in
  `SolidOp::WithTolerance` in place, so `kernel_ogeom::ops::bool_once`
  runs that op's booleans fuzzy at it, through a thread-local
  `ToleranceScope` the chain holds per op), per-feature panel editors (`editors.rs`,
  `editors/`, `datum_panel.rs`, the failing edges' rows in `failing.rs`), declared as panel
  blocks on a `Form` (`form.rs`, over `bench_api::panel::Form`: one editor written for two passes, declaring the widgets
  `task_widgets` hands `bench_panel`, or applying the one event `task_event`
  names, where the widget it names takes the value), the task
  lifecycle (`task.rs`: snapshot on open for OK's recorded call, live
  edits, Cancel left to the host's journal; formulas typed go to the
  feature, their values not written); `build.rs` translates a body's feature
  history into kernel `SolidOp` chains (`BuildPlan` maps op index → feature
  for error attribution). `split.rs`: the body menu's Split… and
  `design.split` make pieces, each a body starting with a Clone of the
  body split and a `Split` keeping one side of a plane
  (`SolidOp::HalfSpace`) or a sketch's profile through the solid; the
  pieces of one split share its `split` number and `split::sync` makes a
  change to one piece's cut in all of them.
  `separate.rs`: Separate pieces (body menu,
  `design.separate`) makes a body per solid of a body's shape
  (`KernelQueries::solid_pieces`), each a Clone of it and a `Piece`
  keeping the solid nearest a point on it (`SolidOp::KeepPiece`).
  `section.rs`: Section to sketch (body menu,
  `design.section`) cuts the body's solid by planes through
  `KernelQueries::section` and keeps each cut as a new sketch, its curves'
  ends joined so it closes.
- `workbenches/wb_surface`: the Surface bench (`docs/SURFACES.md`).
  `SurfaceFeature` (kind `wb.surface`) on surface bodies only:
  `SurfaceWorkbench::takes_surfaces` (a surface body, or one of sketches
  and datums), and Design sends its features off a surface body to a new
  one. `build.rs` turns the body's history into `SolidOp::Surface`
  steps (`kernel_api::SurfaceOp`, `CurveSource`: a sketch's chains from
  `wb_sketch::profile::extract_chains`, open or closed, in the body's
  frame, or a picked `EdgeProbe`); `kernel_ogeom/src/ops/surface.rs`
  builds them, a constructive step adding its sheet beside the body's
  pieces in a compound, Sew joining every face (a closed shell made a
  solid, across a gap when given one), the other steps the kernel's sheet
  operations. A kind the kernel cannot build would carry `waits` (tool
  `planned`, no command), with an ignored test for its gap in
  `tests/kernel/surface_ops.rs` (none today). The free-form scan fit
  (`surface.fit_scan_spline`, `SurfaceOp::FitScattered`: a region flooded
  across triangles turning less than `turn_deg`, its points kept with the
  step) builds through the kernel's `fit_surface_scattered_over`, seeded
  on the nearer of a fitted plane or cylinder. A patch network
  (`surface.fit_network`, `SurfaceOp::FitNetwork`: curves as sources and
  polylines, patches as signed curve numbers or chained sides, the scan's
  points read at plan time) builds through the kernel's `fit_network_chained`,
  every curve sampled to a polyline, the points within the curves' box,
  with optional parameter-correction passes after refinement.
  `surface.fit_scan` fits a plane or cylinder to a scan region (grown
  through `KernelQueries::grow`) and builds it as an extrusion;
  `surface.trim_scan` (`SurfaceOp::TrimByScan`, the scan's triangles in
  the body's frame) cuts the body's sheets by each other with the
  kernel's `split_sheet`, keeps the faces lying on the scan and sews them.
  Commands `surface.*` take `sketches`, `body` (a body or a feature in it)
  and any field by name, refusing a step that misses its inputs
  (`SurfaceFeature::missing`); `surface.set` changes a step's fields and
  curves, `surface.check` returns a body's joins. Check continuity (`surface.check`) asks
  `KernelQueries::continuity` (the kernel's `analyse_blend` over every
  face: gap, crease and curvature jump per shared edge, G2 below
  `G2_PER_MM`) of the selected body and labels the edges through
  its viewport's `labels`. The curvature map and zebra stripes
  (`analysis.rs`) paint the selected body as one viewport mesh
  (`ViewportOutput::meshes`), cached by the body's geometry revision: the map's values from
  `KernelQueries::curvature` at the mesh's vertices, the stripes from its
  normals, each triangle split along the stripes' borders; colours from
  the palette's `analysis_*` and `stripe_*`. Its task panels (`panel.rs`:
  each step's editor, the continuity table, the analyses' settings and
  legend) are declared widgets drawn by `bench_panel`, one
  `panel::Form` walk listing them (`task_widgets`) and, given a
  `PanelEvent`, applying it to the step or display it walks
  (`task_event`).
- `workbenches/wb_assembly`: joints between bodies (`joint.rs`: Mate of two
  planar anchors with offset/flip, Align of two axes, Angle, Hinge, Slider,
  Fixed, Parallel, Perpendicular, Distance and Tangent; Ground keeps a body
  where it is. `JointTool` is the one table of what each tool takes, its
  command, icon and key, and how a joint starts from where the bodies sit:
  an angle or distance at what they make, a slider's or fixed joint's
  relative turn recorded. A hinge or slider carries a `Drive`: its one
  motion (`JointFeature::travel`, degrees from the turn it was made at, or
  mm along the axis) held at a value or kept within limits; `freedom`
  counts a limited motion as free and marks it `at_limit` where the
  joint rests on an end. Anchors are kept in each body's own frame; an
  axis comes from a round face or an edge, a circular edge's axis from
  `EdgeRef::circle`, which the host fits to the outline), interference
  (`interference.rs`: `plan` reads the visible solids and the pairs whose
  placed bounds meet, `Check::run` asks `KernelQueries::overlap`, the
  kernel's common of each pair, measured and meshed, on worker threads
  away from the window with progress and a stop flag, a pair the kernel
  fails on listed as unchecked (`Interference::unchecked`) while the rest
  run; the shared solid
  draws as an `OverlayMesh::on_top`, the renderer's `on_top` pass, blended
  with no depth test),
  dragging (a left press on a body the solver moves takes hold of it
  without consuming the press; moves solve `solve::drag`, the joints plus
  a light pull of the grabbed point toward the cursor on a view-facing
  plane, then the joints alone; with collisions on, `collide.rs` checks
  the way there in steps no longer than half the moved body, a step that
  makes a pair share more material than at the drag's start is refused
  and the pull halved back to contact; the release records `asm.place`,
  carrying in `turns` the coupling counts it changed; Escape, a tool or
  lost focus puts the bodies and counts back; `session.rs` keeps the
  task, picking, held body and handles per tab),
  recording (`sweep_frames` solves a drive's sweep on a document copy;
  the host's `HostRequest::RecordAnimation` draws each frame with the CPU
  preview renderer, framed alike, into an animated PNG, a GIF or numbered PNG frames, `app/animation.rs`;
  `HostRequest::SaveFile` writes any bytes a bench makes where the user
  picks), the
  exploded view (a task that moves bodies and puts them back on close,
  recording nothing), the parts list (`parts.rs`: bodies grouped by
  identical shape snapshot), the solver
  (`solve.rs`: each free body placed on its own against the bodies placed
  before it, turning its first joint's directions into agreement, rings
  taking their turn once most of their joints have something to hold to;
  then every free body refined together by damped least squares over all
  joints, which closes rings; joints still apart are reported by name;
  `freedom` reads each body's remaining motions off its joints' Jacobian,
  cached per edit for the status bar), and task panels for picking,
  joint settings and moving a body by numbers (`panel.rs`, the dispatch,
  with one module per panel or group under `panel/`: `joint`,
  `coupling`, `moving`, `interference`, `explode`, `motion`, `copies`
  (with replacing a body and rigid groups) and `parts` (with mass);
  every panel is one `panel::Form` walk, `<panel>_walk`, declaring the widgets
  `task_widgets` hands `bench_panel` and, given an event in
  `task_event`, naming the edit its field stands for (an `<Panel>Edit`),
  which `<panel>_edit` makes through the bench's own commands,
  `asm.set`, `asm.place` and the rest, where one makes the change; `task_turn`, egui-free like the rest, handles
  OK and Cancel and collects work done away from the window, which
  `busy` keeps frames coming for, a drive's sweep stepping in
  `on_frame`, the parts list's Copy a `HostRequest::CopyText`; only
  `draw_task_panel` touches egui; `panel_tests.rs` checks each against
  its command). Solves run inside the
  gesture that made or edited a joint and record ordinary
  `SetBodyPlacement` ops. Components (`core_document/src/components.rs`: `Component`
  with a parent and `flexible`, `Body.component`, ops `SetComponent` and
  `SetBodyComponent`) nest bodies in the tree (`TreeItemId::Component`);
  the solver holds a rigid one's bodies together where they sit
  (`wb_assembly/src/components.rs`, the joints inside it resting) and
  `move_with_unit` moves them all when one is placed. Couplings (`coupling.rs`, their own kind
  `wb.assembly.coupling`, stored on the driven joint's body) tie two
  hinges or sliders by a `Gearing` (gears, belt, rack and pinion, screw)
  and a ratio, taken from where both joints stand when made (`driver_at`,
  `driven_at`); a `Link` is one residual in the refinement and counts
  against the driven body in `freedom`. A hinge's travel wraps at ±180°,
  so a coupling counts its driver's whole turns (`turns`), continued from
  where each solve starts; `place_bodies` stores the new count
  (`counted_couplings`) with every solve, drag and sweep that crosses the
  wrap.
- `render_wgpu`: data-only renderer (`FrameSubmission` in, pixels out) on
  wgpu: Vulkan, Metal or DirectX 12, the first the platform offers
  (`WGPU_BACKEND` picks one), the GPU chosen by `preferred_gpu` or for
  performance. Shaders are WGSL (`shaders/`), validated by naga in its
  tests. GPU picking with async readback (`readback.rs`: a buffer per pick
  or picture, mapped once its frame is done); mesh cache keyed by the
  body's id, or the shared mesh's key when a `BodySubmission` carries a
  `MeshInstance` (its mesh drawn moved by `Draw.model`), and revision.
  Edges are instanced screen-space quads (`vs_edge`), any
  width on any backend; the clipping plane discards per fragment. The pick
  pass writes depth into a colour target as well, since a depth texture
  cannot be copied in part, and, where the device has
  `Features::PRIMITIVE_INDEX`, the triangle drawn
  (`pick_primitive.wgsl`, `PickResult::triangle` with the revision the
  body was drawn at); the host takes it as the face under the cursor when
  that body was drawn with its document mesh at that revision
  (`DocumentSession::drawn_meshes`, not a textured body), and otherwise
  searches for the triangle nearest the picked point. The window takes a
  plain format, as egui blends for; the scene draws into an sRGB texture
  that the blit encodes onto it.
  A `BodySubmission::scalar` (`ScalarField`: a value per vertex with
  its revision, range, bands, ramp and the colours below, above and for
  `NaN`) paints a body through a ramp: the values upload in the vertex
  colour slot once per revision (an unmeasured one as a flag in the
  slot's second lane, since a GPU compiler may take NaN as equal to
  itself), the rest is per draw (`DrawUniform`,
  the ramp a row of a 256×16 atlas, `ramp.rs`), so recolouring uploads
  nothing; the scene fingerprint hashes it all. The host attaches a
  bench's `ViewportOutput::scalar_maps` (`ScalarMap`, in the document mesh's
  vertex order) to that body's submission in `frame.rs`, and
  `ui/legend.rs` draws the first map's `MapLegend` at the view's bottom
  left.
  `FrameSubmission.side_pane` (`SidePane`: a rect the viewport's size,
  the bodies drawn there instead, and an optional `PaneCamera` of its
  own, hashed in the fingerprint, with its own frame uniform) draws a
  second pane; a pick inside its rect draws its bodies with its camera
  (`Pane::at`). View › Split view puts every body but the scans, and the
  selection and hover paints drawn for them (`overlay_bodies`), in it
  (`DocumentSession::split_view`, the UI halving the viewport); View ›
  Link split cameras off gives it `DocumentSession::side_camera`
  (`CameraController::detached`, kept as `KeptView::side`). Pointer
  input over it (`input.rs::side_pane_input`) runs with the side camera
  swapped in as the session's and the cursor in its pixels, so benches,
  navigation and selection read it as the viewport; overlay hooks get
  `ctx.side_pane` (`SidePaneView`) and place what lies on its bodies
  with `world_to_viewport_on`, marks and labels clipped to both panes.
  Pick pins are `core_document::numbered_pins` (constant size, scaled by
  `pixels_per_point`): the measure tool's picks, Inspection's Shift-click
  seeds and its Align by points pairs (`wb_inspect/src/picks.rs`).
  `FrameSubmission.grids` (`GridSubmission`, `grid.rs`): a patch of any
  plane whose lines `grid.wgsl` works out per pixel, three decades at once,
  each faded by its spacing on screen, depth-tested and never picked.
  `step` is any length (the finest decade); the fade's thresholds come
  from `core_document::grid::FADE_PX` in `fade_px`, which the step
  choice (`finest_step`) and the sketcher's grid snap (`clear_step`, the
  finest decade past half its fade in) read too.
  `app/scene_guides.rs` lays out the ground grid and origin planes; View ›
  Grid and Origin planes turn them on (both start off), and an edit
  session holding the view hides them. A bench's `ViewportOutput::view_grid`
  (`PlaneGrid`: a plane, a base step, colour, opacity and dash; the
  sketcher's on its plane while its grid is on) is drawn in the ground
  grid's place (`scene_guides::bench_grid`); packages have no such hook. While the active bench's
  `shows_origin_planes` is true (the sketcher's plane picker) the planes
  draw whatever the setting, and the one under the cursor (a ray test, a
  body in front of it winning) reaches the bench as
  `ctx.hovered_base_plane`.
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
  `with_tab` turn must not touch it, save one exception: a script's bench
  command run in a tab off screen (`PrintCadApp::visiting`) lends that one
  bench the tab's kept state for the call and hands it back after
  (`execute_command`, `lend_state`/`return_state`). `app/` modules: `frame.rs` (per-frame loop),
  `input.rs` (events, selection), `commands.rs` (UI command application),
  `recompute.rs` (parametric rebuild driver), `links.rs` (parts linked from other
  `.prtcad` files: `Body.link` names the file and body, the shape is read
  from it on a thread and never saved here, a changed modified time marks
  the part stale until Reload), `workbench_host.rs` (ctx
  plumbing), `kernel_worker.rs` (kernel thread, keeps the UI responsive; keeps
  the last few solids per body by what they were built from, so moving a
  body's tip back and forth through its history, or undo and redo, finds
  them built),
  `updates.rs` (the latest release on GitHub, read on a package thread
  at start when `UserSettings.updates.check_at_start` allows, when the
  Preferences › Updates page opens unlooked and from Help › Check for
  updates; it only informs, nothing downloads; a newer release shows
  as a card at the view's bottom right, `hud::draw_release_notice`, until
  put away),
  `session_store.rs` (what a run leaves the next, in the configuration
  folder beside `chats.json`, never in the document: each file's view in
  `views.json`, the latest 50, put back on open in place of both fits;
  the open files, window place and panel layout in `session.json`,
  both kept a second after they last changed (`drive_session_keeping`,
  so a killed run leaves them) and on the exit paths after the saves,
  never by a run a bench hook opened, the files reopened at start
  unless a bench hook or the command line's files open something, as
  `UserSettings.session.reopen_documents` allows; desktop only;
  `drive_bench_settings_keeping` writes a bench's settings changed outside
  Preferences, such as the sketcher panel's grid and snap switches, into
  `UserSettings.workbenches` and the settings file, as a Preferences commit
  does),
  `sixdof.rs` (6-DoF mouse reader thread; holds the puck's current deflection,
  which `camera::apply_device_motion` integrates once per frame).
  `ui/` is one module per region: `menu_bar` (the menus, New/Open/Save
  and Undo/Redo buttons beside the app mark, the tool search at the
  right), `toolbar` (groups: the bench switcher and each run of a bench
  category along `ToolDescriptor.row`; each dragged by its grip along a
  row, to another or to a new one, kept in `UserSettings.toolbars`, a
  group it does not list placed where its bench puts it; variant
  dropdowns), `combo_view` (tree +
  `property_panel`), `feature_tree`, `task_panel` (host of the workbench
  task under one header: its title, `TaskInfo::subtitle`, a pill for
  `TaskInfo::status`, Cancel and Done; Enter is Done; Escape stays with
  the bench and never ends a task), `status_bar` (ending in the log, console
  and assistant switches and Recompute), `view_toolbar` (floating
  pill, with Measure, Print bed and Annotations; in perspective it carries the field of view, dragged or typed,
  which `CameraController::set_field_of_view` changes keeping the framing), `hud` (workbench HUD corners, hover card), `overlays`
  (line/mark/label painters), `start_page`, `preferences` (modal on a
  draft `UserSettings`, committed by `CommitSettings`), `command_palette`
  (Ctrl+K), `step_import_modal`, `log_view`, `host_ctx`.
  Every menu's rows are egui buttons (hover and disabled states come
  with them), inside `ui_kit::widgets::fitted_menu`, which scrolls what
  would run past the window's bottom. The scene hovers and picks only
  while the pointer is on it, not on a menu or card over it. An edit
  session on a plane opening clears the view's selection.
  Row menus (`feature_tree/menus.rs`, `context_menu.rs`, the shared
  `body_menu.rs`) are flat; what needs numbers or choices is an
  application task (`host_tasks.rs`: Placement, Appearance, History,
  the note editor, Mark surface and the print layout's panel;
  `array_task.rs`, `texture_task.rs`), held on `UiLayer` for its tab and
  drawn in the task panel while no bench task is open, editing the
  document live as a bench task does and closing through `TaskClosed`
  plus the `Recorded` calls it would make. Each walks its panel once
  through a `bench_api::panel::Form` (`walk`, the helpers in
  `ui/task_blocks.rs`): declaring the blocks (`widgets`), drawn by
  `bench_panel` as a bench's panel is (a tight `Stack`), or
  handing a `PanelEvent` it gets back to the field it names (`event`),
  which edits through the same document mutators and `UiCommand`s.
  Closing a tab, the window or the app with unsaved edits asks in the
  window (`app/unsaved.rs`, `ui/unsaved_modal.rs`) and the close waits for
  the answer; quitting asks for each tab with edits in turn.

The `Workbench` trait is the only seam between the host and a bench; the
host never names a bench: `scripts/check-workbench-seam.mjs` checks
`crates/app_shell/src` and its manifest, discovering built-in bench crates
and concrete workbench types from their source directories. `descriptor()` says
what a bench is: `icon`, the `feature_kinds` it claims (the
`FeatureNode::workbench_id` values it presents, edits, renders, picks and
deletes; Design claims `core.datum` too; a kind claimed twice fails
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
`floating_card()` + `card_event()` put declared widgets in a card over the
view beside what they edit, for a quick edit of a value or two (a sketch
dimension, a corner's size; anything more is a task): the host draws it
(`ui/floating_card.rs`, through `bench_panel`, the `focus` field
given the keys with its text selected), Enter or OK or a click elsewhere
in the view is `CardClose { accept: true }` (held back while `refusal`),
Escape or Cancel `accept: false`; a card with a `hint` is a readout (the
sketcher's drawing values, `ViewportHud::card`, `Quantity::shown` and
`held`): the hint in place of OK and Cancel, the keys and pointer left
with the bench, nothing held, and only while no other card shows; `app/cards.rs` closes the step before
it as it opens, holds the journal while it is open, closes one step on
OK and undoes what was written on Cancel, so a bench keeps the edit as a
draft until OK;
`viewport()`, asked once a frame under one context, returns a
`ViewportOutput` (meshes in the scene, the screen-space `overlays`,
`polygons`, `images`, `marks` and `labels`, `hud`, `scalar_maps`,
`status`, `clip_plane`, `view_grid`, `faded`; a bench fills what it draws
and leaves the rest at its default) and, with `task()` and
`editing_feature()`, feeds the viewport and chrome (polygons are filled fans drawn beneath the lines: the Assembly's
  Move handles, `wb_assembly/src/handles.rs` over `transform_gizmo`);
`WorkbenchRuntimeContext::pixels_per_point` carries the UI scale: input and
overlay coordinates are physical pixels, while the transform gizmo's adapter
converts them to and from logical pixels. `cancel_pointer_gesture()` restores
a held pointer gesture when the window loses focus (before UI event
filtering) or its tab leaves the screen, called from the one place
(`app/gesture.rs::end_gestures`) that also ends the host's own drags,
which live in `DocumentSession::gestures` (a box, a marking stroke, a
scalar handle, the radial menu's press) and end on Escape and task close too;
`ui_settings()` draws the bench's Preferences page (a rail entry for
each bench whose `has_settings` is true; a package's is true when its
settings page has widgets); `feature_info`/`passive_geometry`/`pick_feature`/
`delete_feature`/`property_hints` answer for the feature kinds a bench
claims; `feature_plane` is the plane an owned feature offers to sketch on (an
Inspection section or fitted plane), which the host hands the active bench
as `ctx.active_object_plane` for the tree's selected feature, and the
sketcher's Create sketch takes; `report` is a bench's part of a printed report (`BenchReport`: a title,
summary lines, `ReportTable`s, bodies to picture), which `doc.report_pdf`
lays out on A4 with three CPU-drawn views (`doc_render::pdf`, a small
writer with the standard Helvetica faces); `consumed_feature` is the feature an owned feature's row sits under
(a pad under its sketch; by default its one dependency), every other
feature at its body's top level in build order; `linked_features` names features of other bodies the tree lists
under a body too (a joint under the body it holds to); `derive_on_geometry`
brings a feature's working data up to the bodies' geometry during
evaluation (an assembly joint re-finds its named faces on a rebuilt body,
and `values_moved` re-solves); `tool_history` answers a bench's
`CloseToolGesture`/`UndoToolGesture` (`ToolHistoryReply`, Tasks and
undo); the viewport's `faded`
draws bodies translucent, still pickable, while a tool wants them seen
past (the Assembly's picking); `not_printed` names bodies an export of
everything and the slicer leave out (bought parts); `edit_feature` is
the double click that opens a feature's task
(selecting one never does, so a feature stays selected after its task
closes); `references`/`set_reference` offer the inputs the property
panel's Inputs group swaps (Design's profile, kept out of its task); `busy` keeps frames coming while a bench's work runs away from
the window; `rebuild_jobs`/`invalidate_body`/`invalidate_all` drive solids;
`menu_items`/`on_command` add entries to the viewport body menu, tree rows
and the start page's New cards; `register_import` (a `FileImport`: label,
extensions, one of the bench's own commands) puts a file kind in File ›
Import, and a picked file of that kind runs the command with its `path`
(`import_with_bench`, reached from `import_step_at`), recorded as that
command (the sketcher's `sketch.import_dxf`, read by the kernel through
`KernelQueries::read_dxf`). `docs/WORKBENCH_GUIDE.md` is the
walkthrough. Colors reach the workbenches through
`WorkbenchRuntimeContext.sketch_palette`, never as literals.

**Placeholders.** Everything the design shows is built. The sketcher's
external geometry (`external.rs`) takes solid edges picked while its tool
is armed (clicks fall through to the host's edge picking), or a face for
every edge around it (`KernelQueries::face_edges`: a point halfway along
each, its seam left out), projects each
through `ctx.kernel` (`kernel_api::KernelQueries`, the host hands benches
`kernel_ogeom::QUERIES`) onto the sketch plane in the edge body's frame,
and stores the result as geometry marked in `Sketch::external` with its
`ExternalSource`: pinned in the solver, drawn with the closed sketch as
drawn geometry is, left out of profiles until marked as counting (`ExternalSource::defining`, which the
Construction button switches as it does construction for drawn curves),
drawn in the external colour (dashed while a guide, solid once it
counts), never dragged, and projected again: when the sketch opens, and
as derived values whenever its solid changes (`derive_on_geometry`, given
the kernel `DocumentService::set_kernel` holds, asked on the UI thread
once per change of what it projects), the edge found by its faces' names
(`ExternalSource::faces`, read from the mesh as it is added) before its
point. From the sketch's own body it is the solid as it stands at the
sketch's place in the history: Design's plan asks it
(`BuildPlan::projections`, `kernel_api::ChainProjection`, answered by the
chain beside its probes in `SolidBuildResult::projections`, kept by
`Document::store_projections`), so what the sketch builds never moves it;
another body's solid built from the sketch waits for the sketch to open. New projections take the
Construction mode (a guide in it, counting out of it; the commands' `counts`
flag, off unless given); projected ends at one spot join in the profile
once per editing session (in place when the curve is the same kind). Its
intersection variant takes picked faces instead and adds the curves where
each crosses the sketch plane (`KernelQueries::section_face`, the face's
solid sectioned by the plane's half space and the edges on the face kept;
`ExternalSource::section`), refreshed the same way. File › Export
(`app/export.rs` over `kernel_ogeom::export`) writes the visible or the
selected bodies as STEP, or as STL or 3MF meshed afresh at the dialog's
tolerance and welded closed, on a thread of its own. The registry gathers
`Workbench::semantic_pmi` (`kernel_api::SemanticPmi`, re-exported by
`core_document`); `app/export.rs::export_pmi` takes its world probes into
each body's frame, and `kernel_ogeom::export::export_with_annotations`
writes AP242 tolerances, datums and dimensions on the probed faces beside
the drawn notes. Annotated bodies have distinct parts, and datum letters
are disambiguated across parts; the start page's
Export for printing walks it on the pocketed example. With
`PrintingSettings::apply_compensation`, mesh exports scale each body about
its middle by `compensation` (`ExportBody::scale`; `app.print_compensation`,
from `inspect.compensation`). File › Send to
slicer writes the visible bodies to `$TMPDIR/printcad/slicer/` in the
format of `PrintingSettings::slicer_format` and runs
`slicer_command` on it (`{file}` places the path, else it goes last;
empty uses `xdg-open`). File › Print layout (`app/print_layout.rs`, a
task, its packing `doc_render::print_layout`) lays each part on its largest face it stands on and packs the
copies on the bed, plate after plate, without moving the bodies; the
counts come from `Workbench::print_parts` (the Assembly's parts list),
and export, the slicer and `doc.print_layout` take the layout. The
parts list's print counts, volume and filament mass are in
`docs/PRINTING.md`, with the Hole's nut trap. A save carries a
CPU-rendered preview (`doc_render::thumbnail`, in the save worker) as the
container's first entry, `thumbnail.png`, which
`Document::read_thumbnail` reads without unpacking the rest; the recent
cards show it. What's new reads `crates/app_shell/RELEASE_NOTES.md`, and
a test fails when the running version has no entry there. The Edit menu's Cut/Copy/Paste go to the
active bench as `MenuScope::EditMenu` commands (the sketcher keeps a
geometry clipboard); the view toolbar's clipping plane (`camera/section.rs`, per tab, a
plane square to X, Y, Z or the view, on a flat face or a datum plane, or
through three points, with an offset along its normal that a formula may
set and a drag handle, `app/section.rs`, and bodies it leaves uncut,
`SectionLook::uncut`, which every pass and the CPU picks skip the clip
for; a wedge adds `FrameSubmission::clip_wedge`, cutting only what lies
behind both planes, `section::Clip` on the CPU side), or while the active bench returns one the
plane of any direction `ViewportOutput::clip_plane` gives (the sketcher's
per-sketch section view), reaches the renderer as
`FrameSubmission.clip_plane`: every scene shader and the pick pass write
a clip distance, the cut's back faces draw as a flat darker section
(a body's `SectionLook`: `open` for a mesh that does not close,
`scans::closes` per revision, whose inside draws lit as a shell, and a
`hatch` angle per cap when several closed bodies are cut), and
CPU edge picking skips what it hides; the view pill's Measure arms a readout
drawn over the scene (`app/measure.rs`: a point, an edge or a face picked
gives a length, radius or area, two give the distance and angle between
them; Escape puts it away); the print bed is a
line box from the Printing preferences. The mockup shows Design and Sketcher elements the app
does not implement yet. They stay on screen as disabled controls with a
`// PLANNED: <what it does when built>` comment next to them and a
`ToolDescriptor::planned(note)` on tools (`tool_button` renders them dim,
the host never dispatches a planned id). Nothing outside those two
workbenches gets a placeholder.

**Linked copies.** A body with `copy_of` (op `CreateLinkedCopy`,
`Document::create_linked_copy`) takes its source's shape: its geometry,
snapshot and face colours are derived from the source's
(`refresh_copy`, run whenever the source's geometry is set or dropped and
after a load; a copy's snapshot is not saved), placed by its own
placement. It takes no features (`body_solid_is_imported` is true for it,
Design's target body skips it). A mirrored copy (`mirror`, a plane in
the source's frame) draws the source's mesh mirrored at once and waits for
its snapshot, the kernel's mirror of the source's (`KernelQueries::mirror`),
which `drive_mirrored_copies` asks the kernel worker for
(`copies_awaiting_shape`, kept by `set_mirrored_shape` only while the
source still has the snapshot it was made from). Arrays of linked copies
(`core_document/src/arrays.rs`: `CopyArray`, a source body or a
component, an `ArrayPattern` (linear, polar, a path's stations or a
sketch's points, in the document's axes), a count and skipped
occurrences, kept in `Document::copy_arrays`) own their copies: one op,
`SetCopyArray`, carries the whole array with its copies' ids resolved,
its apply making missing copies, dropping unlisted ones and placing each
at `pattern.motion(index).after(placement of what it copies)`; moving
the source re-places them inside `SetBodyPlacement`'s apply, and
`remove_body` on a copy skips its occurrence instead. A component
array's occurrence copies every body of it (`ArrayInstance::members`)
into a rigid component of its own (`group`). A path or point pattern
carries the stations or points it was read with: the host reads them
(`app/array_paths.rs`: a bench's `Workbench::feature_paths`, chains and
free points, through the kernel's `wire_frames`, or a solid edge through
`edge_frames`) whenever the array is made or edited. The body menu's
Array of copies… is `ui/array_task.rs` (a component's tree row too;
a row's spacing drags in the view, `app/array_handle.rs`); commands
`doc.array` and `doc.set_array`. A copy that draws its source's mesh as
it is submits that shared mesh with its placement as a
`render_wgpu::MeshInstance`, so the GPU holds it once.

**Notes.** Text on a face, an edge or a point of a body, drawn as a
leader to a label (`core_document/src/notes.rs`: `Note` with `id`,
`number`, `text` (`=` a formula), `at` and `label` in the body's frame,
`face` (a `FaceKey`) or `edge` (an `EdgeKey`: its two faces' names and
index) or neither for a point, `author`, `asks`, `resolved`, `hidden`,
`created_at`, and `from_file`, set on reading a file; `Body::notes`, op
`SetBodyNotes`, which carries `Document::notes_numbered`, the highest
number given, so a note taken away leaves its number unused and undoing
an add takes it back; a file whose notes have no numbers is numbered on
reading). `Note::placed` finds the face or edge again on the body's mesh
as built and takes the point and label to it, or says it is lost. The
view toolbar's Note (`view.note`) arms `app/note_tool.rs` (`note_click`,
the card's state `NoteCard`, `note_json`, `hidden_by_part`: a ray from the
view to the point against the body's mesh, so a note the part hides
draws faint and dashed); the card is `ui/note_card.rs`, the task panel's
editor `host_tasks::NoteTask`, the drawing `app/annotations.rs`, rows the
body's Annotations group with Lost and `?` chips. Commands `doc.note_add`,
`doc.note_set`, `doc.notes`, `doc.notes_open` and `doc.note_remove`,
which agents may not run; an agent's `note_add`, `note_set`, `delete` and
`rename` calls carry `author = "agent"` (set in `script_event`), and
`user_note_guard` refuses an agent's rewording or deleting of a user's
note. The `context` tool lists the open ones, `view` draws them with
`notes`, and the chat's `@` list offers them (`MentionSource::Note`, `@#3`).

**Mesh starts.** A mesh body's mesh history (RFC 0004 milestone 5)
builds from the mesh as imported, kept apart as its start
(`Document::set_mesh_start`, op `SetMeshStart`, saved as
`mesh/<body>.start.bin`); the edited mesh the body draws is derived.
`KernelQueries::build_mesh_chain` applies `kernel_api::MeshOp`s
(`kernel_ogeom/src/mesh_edit.rs`: the start welded once into one
`ogeom::mesh::Editable`, so a `MeshRegion::Vertices` marking of the start
still names its faces after earlier steps), a failing step reported with
the mesh before it.

**Base solids.** An imported, converted or repaired solid takes features
by becoming its body's base: `Document::set_body_base` (op `SetBodyBase`,
its own inverse with `on` flipped) copies the body's solid into
`base_solids` (saved as `brep/<body>.base.bin`/`.colors`, the geometry in
the body's own frame) and `body_solid_is_imported` turns false; dropping
it puts the base back as the body's shape. Design's `DesignFeature::Base`
builds as `SolidOp::Shape` of it and must be first. `take_base` (from
`create_feature` and the borrow command) makes a body based on its first
Design feature, putting the Base first in its history; that tool's Cancel
undoes it, and the Base deletes only once nothing follows it. Repair on a
based body mends the base (`set_base_solid`, derived) and invalidates the
body; `Document::body_health` reads the base's findings where there is
one. The tree puts an imported part's features under its row.
Replace shape… (body menus, `doc.replace_shape`) reads a body's shape
from another file: op `ReplaceBodyShape` carries the file as an asset
(a history barrier, as an import is) and sets `Body.shape_asset`;
`bodies_awaiting_shape` lists bodies whose base (or shape, without a
history) is not yet from that asset, and the host's
`drive_shape_replacements` reads each on the kernel thread
(`OgeomKernel::read_solid`: a STEP/IGES file's first body, a `.brep`
file's shape, a mesh file converted) into the base, invalidating the
body, or into its shape (`recompute::land_shape`, which headless runs
too). `Document::shape_body_from_file` records the same op for a body
holding nothing yet. A stand-in (`Body.stand_in`, op `SetBodyStandIn`,
also a barrier) is a shape asset kept for a body built from features:
`stand_in_due` while its feature is the tip and that feature's build
failed, the same drive reads it into the body's shape, and the next
build that succeeds replaces it (`body_solid_is_imported` ignores it, so
the body still builds). The project import (`project_tools`) gives every
body its tip's saved shape as one, and brings what it cannot rebuild in
as `shape_body_from_file` bodies, the saved `.brp` restated for the
kernel's reader by `project_archive::restate`.
Design's Delete Faces (`DesignFeature::DeleteFaces`, `SolidOp::RemoveFaces`)
takes picked faces away through the kernel's `remove_faces`, the
neighbours closing the openings; Offset Faces and Move Faces
(`SolidOp::OffsetFaces`/`MoveFaces`, a distance, or a translation and a
turn about an axis as a rigid row-major matrix) go through its
`offset_faces`/`move_faces`, the neighbours following on their own
surfaces. Offset Solid (`SolidOp::OffsetSolid`) moves every face through
its `offset_shape`, which refuses free-form faces. Scale (`SolidOp::Scale`,
factors per axis about a point or the bounding box's middle) goes through
its `general_transformed_shape` (`ops::pattern::scaled`). Recognize holes (`design.recognize_holes`,
`wb_design/src/recognize.rs`) asks `KernelQueries::recognize_holes`
(`kernel_ogeom/src/holes.rs`: concave cylinder faces grouped by axis and
radius into whole bores, each end an opening, a flat bottom or a coaxial
cone, from the faces sharing its edges) and adds one Delete Faces for
every hole's faces, then per set of alike holes a hidden sketch of their
centres on the opening plane and a Hole feature.
Design's Project (`DesignFeature::Project`, `SolidOp::Project`,
`kernel_ogeom/src/ops/project.rs`) carries a sketch along its normal onto
picked faces: `split_face` (`Projection::Along`) splits them, and for
Emboss/Deboss each piece whose inner point falls in a profile region is
thickened (`make_thick_sheet`, the side checked by a point) and fused on or
cut away; the kernel's `offset_faces` refuses such regions. Its `wrap`
builds `SolidOp::Wrap` instead (`ops/wrap.rs`, `Development`): the sketch
unrolled onto the faces' cylinder or cone from where its middle lands, or
on any other surface laid as the kernel's `SurfaceSheet` (geodesic polar
coordinates from there, its `+x` the sketch's x; a point carried back by
Gauss-Newton on `chart_point` for Emboss/Deboss), each curve a
`Curve::OnSurface` of its chart curve (on a cylinder the curve's exact
B-spline with mapped control points, on a cone a fit, on a sheet the
kernel's `chart_edges`, cut at a pole), cut at derivative jumps, carrying
that chart curve as its pcurve on each picked face's surface, then
`split_face` with `Projection::OnFace`.

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
across the registry) in the order the generated reference lists them (the
Sketcher and Assembly, whose commands live in `commands/<family>.rs`,
sort theirs by the `ORDER` list in `commands.rs`, which a test holds
complete) and runs them in `Workbench::run_command` with the
same context its tools get. Assembly's families each fill a
`CommandTable` (`core_document::command`: every spec beside the function
that runs it, so an id is written once), from which its registration
order, `owns` and dispatch derive; the other benches' families still
pair an `owns` match with their specs. `command::object` turns a JSON
value into arguments. Benches' tests run commands through
`core_document::testing` (the `test-support` feature, turned on only by
dev-dependencies): `ctx(doc)`, a context with no camera, and
`call_checked(bench, doc, id, args)`, a call checked against the bench's
registered spec as a script's is. The app's own are `doc.*` plus every keymap
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
step. The thread wakes the loop through `AppEvent::Script` and a busy A run that stops with an error or is stopped is taken back
(`OpJournal::roll_back`), leaving no undo step; an import inside it is a
barrier, so only what followed it is taken back.
thread counts as async work. Stop sets the engine's stop flag.
A run's chunk value comes back as `RunOutput::returned` (JSON: a table
with keys 1 to n a list, any other an object); the console shows it and
an agent's `lua` answers `{returned, printed}` (`script_answer`).
`doc.faces`/`doc.edges` (`app/edges.rs::edges_of`: kind, a point halfway
along and its direction, length, the faces' indices and names as strings)
read geometry where the body sits, the picks the edge-taking commands use.
Tools end in commands, which is what recording rests on: a bench calls
`ctx.record(id, args, result)` where a UI action ends in the code its
command runs (`HookOutcome.recorded`, carried through `PanelWriteback`
for panel hooks). The sketcher's click is `step::click`, shared with
`sketch.draw` (a shape is one call, flushed when the tool returns to rest);
drags are `step::drag`; Design records in `record_task` when a task
closes, diffing against what the command makes alone (`default_feature`);
Assembly in `record_joint`. The host maps its own UI commands in
`recorded_of` and keeps calls in `PrintCadApp.recording`
(`scripting::Recorder`, which names results and writes numbers exactly,
a double that is exactly a single as the single's digits, so a replay
meets stored values bit for bit); nothing records
while a script runs. The app's own
commands: `doc.*` (the pure ones in `scripts::document_command`, shared
with `headless.rs`), `app.*`, and every keymap command, the file ones
taking a `path` to skip their dialog. Script files: `script_library.rs`
reads `settings::scripts_dir()` every 2 s into `script.<name>` keymap
bindings (`Target::Script`), the Scripts menu, toolbar button and palette.
`printcad --script` (`headless.rs`) runs before the event loop, rebuilds
on the calling thread, logs to stderr. `docs/SCRIPTING.md`'s command
reference is generated; a test fails when it drifts
(`PRINTCAD_WRITE_DOCS=1` rewrites it). A `CommandSpec` carries `notes`,
`examples` (Lua ending in `assert`s) and `see_also` (`.note`, `.example`,
`.see_also`; a package's `bench_api::Command` the same), which the
reference prints. The largest families write theirs as markdown beside
the code (`app/scripts/catalog.md`, Design's `commands/features.md`,
Surface's `commands/steps.md`, Assembly's `commands/joints.md`: a
`## id` section per command, or one several commands share, its
paragraphs notes, a `See also:` line, `### title` and a ```lua block per
example, a `{{name}}` line a shared piece of setup), read once through
`core_document::CommandProse` and added to the specs by id after what
the code adds (`explain`, `CommandTable::explain`); ids, parameters,
`read_only`, agent access and any prose worked out in code stay in the
code, and each family's test holds every section to a registered
command (`testing::assert_prose_names_commands`);
`every_command_example_and_recipe_runs`
(`app/scripts.rs`) runs every example and every ```lua block of
`docs/recipes/*.md` from an empty document through the headless path
(`headless::run_in_empty_document`).
Commands never open a task; Design's make features through
`create_feature`, the toolbar's own path, then merge named fields into the
feature's JSON. `kernel_ogeom/tests/kernel/scripted_part.rs` runs a script through
the real benches to a solid.

**AI agents.** The `agents` crate knows no command either: `rpc`
(newline-delimited JSON-RPC), `acp` (`AgentChat`, a worker thread per
agent process speaking the Agent Client Protocol: `ChatCommand` in,
`ChatEvent` out), `mcp` (the server core over a `ToolHost`) and `bridge`
(`printcad --mcp`, relaying stdio to the app's socket with a header naming
the chat), and `discovery` (BM25 `search`, loose `describe` and the
one-line-per-entry `index` over plain `Entry`s, with the CAD `SYNONYMS`
table, pinned by the ranking test in `app/discovery.rs`, which builds the
entries from every command through `scripting::command_entry` and from
the guides' sections and `docs/recipes/`; the index goes into the
instructions, and the console's `help(word)` searches the same way).
`app/mcp.rs` listens on `$XDG_RUNTIME_DIR/printcad/mcp-<pid>.sock`,
one thread per client handing each tool call to the UI thread
(`drive_agent_tools`); `call` and `lua` run as script-thread jobs
(`Job::Command`/`Job::Script`, `RunKind::Agent` answering the tool), so
tab pinning, one undo step per call and Stop come from the script path.
A change waits in `PrintCadApp.approvals` while its chat asks
(`asks_before_changes`); `CommandSpec::read_only` (declared by whoever
registers the command) is what never waits, and `CommandSpec::agent`
(`AgentAccess`) what an agent may never run (`app.quit`, `tab.close`,
`doc.set_agent_rules`) or must always have allowed, one `call` at a time
(files, import/export, slicer, undo/redo, tabs); `agent_check` enforces it
for `call` before it is held and for every command a `lua` script calls.
`app/agent_context.rs` is what an agent is told: the MCP instructions,
built per connection on the UI thread (the relay asks for them, and for
the `printcad://rules` and `printcad://context` resources, with requests
no client can name), the `context` tool, the prompts, and the user's rules
(`AiSettings.rules` for every document, `Document::agent_rules` saved with
one, op `SetAgentRules`), which also go with a chat's first prompt and with
the next one after they change. `app/chats.rs` keeps
`PrintCadApp.chats`, each started with the relay as its MCP server
(every tool `always_load`, sent as `_meta."anthropic/alwaysLoad"`, so a
client that defers tools behind a search has them in its first turn;
`read_only` becomes `readOnlyHint`);
`ui/assistant.rs` draws them and answers with `UiCommand`s (the agent's
text as markdown, a copy button on a hovered message, the header's jumps
between the user's messages kept in `ChatScroll` from the last frame's
layout); a message
sent while the agent is on a turn waits in `Chat::queued` (edited or
dropped from the panel) and goes when the chat is ready again, one per
turn, the rules added as it goes; Stop sets `Chat::held` until the user
sends again or resumes; a call of
printCAD's own tools draws as `agent_context::tool_label` (the call's
`description` argument, which the instructions ask for, else the command
or the script it runs), from the arguments ACP sends as `rawInput`. A chat belongs
to the tab it started in (`Chat::tab`; its tool calls run there through
`submit_script_in`), and once it has a session id and the tab a file,
`persist_chats` keeps `{agent, session, title}` with the file in the
app's `chats.json` (`app/chat_store.rs`); opening the file
(`restore_chats`) brings them back `Resting`, and the first time one is
shown `wake_chat` starts its agent with `session/load`
(`AgentChat::start`'s `resume`), which replays the conversation. The agent's
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
drops on X11 only). Typing `@` in the box completes mentions
(`app/mentions.rs`, through `ui_kit::completion::Completion`, the
formula fields' dropdown made general): every open file and the chat
tab's selection, `PrintCadApp::mention_options`. A pick puts its token in
the text, drawn as a chip by the box's layouter and taken away whole by
Backspace or Delete, and records `UiCommand::AddMention`, whose block
(`mention_block`: a file's bodies and features, a selection's faces with
`FaceRef::surface` and `Document::face_maker`, edges, bodies and tree
features) is taken then into `Chat::mentions`; a message keeps the ones
its text holds (`mentions::in_text`) through `Queued` and
`ChatEntry::User`, and each goes as an `Attachment::Text` block,
`<printcad:mention>{json}</printcad:mention>`, which `take_blocks` reads
back out of a replayed message. A new kind of mention is a variant of
`MentionSource`. `context`, `view`, `call`, `batch` and `lua` take `file`
(a path, file name or tab title, `target_tab`) to read another open
file; there only `read_only` commands run (`RunKind::Agent::elsewhere`,
checked per call, `changes_elsewhere`). The `view` tool and `doc.picture` (headless too)
draw through `doc_render::proof`: a `Request` (view by name, angles or direction
in the axis preset's frame, bodies, highlight, markers, section, edges,
xray, size, annotate) over `thumbnail::draw` and its `Look`, framed to
what is drawn, labels as SVG through usvg with only the bundled Plex
face (the file `ui_kit::theme::SANS_REGULAR_TTF` bundles); every view but `current`
draws the same bytes whatever the camera. Agents are configured in `UserSettings.ai`. `docs/AI.md` is the user guide. `docs/ASSEMBLY.md` is the Assembly user guide.

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
dirty without marking the document edited. A slot whose formula fails
stops its feature's build with the formula's error
(`Document::build_formula_error`, `BuildPlan::stop_at_failing_formulas`),
rather than build from the number its data held before. Results a bench records rather
than derives follow through `Workbench::values_moved` (the assembly
re-solves placements): the host calls it from `settle_formulas`, every
frame and in `close_gesture`, which every undo boundary goes through, so
an edit and what it moves are one step; headless runs settle after every
command. A value set by hand goes through
`DocumentService::set_parameter_value`. The UI: `ui_kit::widgets::
FormulaField` over `bench_panel::DocumentFormulas` (the document's last
values) in the property panel's Parameters group, Design's and the
Assembly's task fields and the sketcher's dimensions (bound ones drawn in
`SketchPalette::formula`); a selected variable set or the configurations
table shows its editor in the Data tab (`ui/variables_view.rs`), and the
tree's document row makes them. Commands: `var.*`, `config.*`,
`doc.parameters`, `doc.set_formula`, `doc.set_value`. `docs/VARIABLES.md`
is the guide.

**Tasks and undo.** A feature edit is a task in the right panel: edits apply
live, OK accepts, Cancel takes them back. `app/task_step.rs` opens the
task in the journal (`OpJournal::begin_task`) at the end of the frame a
task is drawn or the active bench answers one (a tool's feature made
after the panel was drawn goes in with it), and no boundary closes a step
until it ends: `TaskClosed` accepted closes one step named by the task
(`accept_task`); closed by the header's Cancel (`take_back`, only a
confirmable task has it) the journal takes back everything done while it
was open and leaves no step (`cancel_task`, `roll_back`'s barrier rule:
across an import, asset or repair only what followed it, said in the
log); a task gone without closing, or a non-confirmable panel's close,
keeps its edits as a step. Benches and the app's tasks restore only their
own state on Cancel (selection, picks, the tool). A task that records
calls mid-way (the Assembly's joint turned) asks
`HostRequest::KeepTaskEdits` so Cancel stops there.
`history::assert_cancel_takes_back` is the tasks' shared test. A `TaskInfo::stepwise` task (the
sketcher's editing session) is the exception: each edit in it is an undo
step of its own. A tool can make its gesture one step before the pointer
is let go and take back exactly that step: every closed step has a
`JournalEntryId` (`OpJournal::note` answers it, `undo_entry_id` names the
newest, kept through undo and redo), the hook context carries the newest
as `ctx.undo_entry`, `HostRequest::CloseToolGesture` closes the step
through `close_gesture` and `UndoToolGesture` undoes `entry`
(`OpJournal::undo_entry`, refused unless it is the newest step, while a
script holds the journal, or while undo itself is blocked), each answered
to the bench at once through `Workbench::tool_history`.

**Feature preview.** While a bench's `editing_feature` is one that builds
solid (Design's open task), `recompute.rs` asks its body's builds for a
preview (`request_build_solid`'s `preview`, `ChainRequest::preview`
over the feature's op range): the chain keeps the feature's tool and the
body before it (an adding feature) or after it (a cutting one) in
`SolidBuildResult::preview`. The body's geometry is then that solid, so
picks match what is drawn; `session.previews` keeps the whole solid, put
back when the task closes (and in the save snapshot), and the tool, drawn
`front_only` in `rendering.preview_color`/`preview_opacity` with its edges
over its faces (the renderer's `TranslucentFront` pass, back faces culled).
A previewed fillet or chamfer also has the chain work out the largest size
that builds (`ops::dressup::size_limit`, kept in the `ChainCache` by the
history before it and `SolidOp::size_limit_key`), answered as
`SolidBuildResult::size_limit` and kept by `Document::store_size_limit`;
Design's `size_limit.rs` reads it while the key holds, and its handle
(`Handle::stop`) and fields (`Quantity::drag_max`) stop a drag there.

**Drags stop at limits.** A drag never carries a number past its limit;
only typing may. In the view, `core_document::steps::nearest` steps a
dragged number and lands on a limit that is no whole step; in fields,
`ui_kit::widgets::{hold_dragged, step_within}` hold a drag and the wheel
to `QtyField::drag_range`/`FormulaField::drag_range`, which a declared
`Quantity`'s `min`/`max` and `drag_min`/`drag_max` set. The property
panel's parameter rows take a bench's `Workbench::parameter_drag_range`;
the app's scalar arrows (`app/scalar_arrow.rs`) carry `ScalarDrag::stops`
(the clipping plane's scene span).

Recompute loop: workbench edits document → features marked dirty via the
dependency DAG → `drive_part_recompute` (each frame) asks every bench for
its `rebuild_jobs` (a `BuildPlan` of `SolidOp`s per body, the bench settling
the dirty flags of the plan's features and inputs itself) → kernel worker
thread → results land in the document's imported-geometry sidecar →
rendered/picked like any body. A failure leaves the body its history
before the failing feature: a plan that cannot plan a feature stops there
(`BuildPlan::failed`, the features after it in `unbuilt`; Design's
`body_plan` and Surface's `plan_until_failure`, beside the strict
`body_build_ops`/`body_plan` that answer an error), and a kernel failure
at op N has the worker build the ops before the failing feature's first
(`run_chain`, `KernelResponse::SolidBuilt::failed`); the failing feature
carries the error and every feature after it "not built"
(`mark_unbuilt`), and a first feature failing leaves no built shape
(`SolidFailed::nothing_built`). A body has one build out at a time
(`session.builds_out`, `recompute::BuildsOut`): a plan made while it builds waits in its
place, a newer one replacing it, and goes when the build lands
(`build_landed`), so a drag that changes a feature every frame builds only
the latest shape rather than replaying each step. A history that changed shape goes through
`invalidate_body`; a history jump or Recompute All through `invalidate_all`.
Builds skip what an edit did not change (what is left is in `docs/ROADMAP.md`):
`kernel_ogeom::ChainCache` (one per body on the worker) keeps the chain's
state (model clone, solid, face names, pattern tools, probe answers) at the
start of the op that differed from the last build and at the end, and the
next build whose ops agree up to there resumes from it; an edited op that
makes the same solid as before (`fingerprint`: geometry, not snapshot text)
returns the last result; faces and edges whose geometry, deflection and
chords are unchanged keep their triangulation, outline and bounds
(`reuse.rs`, keyed by geometry since every op renumbers its faces). The
app drops a build a newer plan replaced (`KernelWorker::drop_build`: a
queued one never runs, a running one stops unless past half its body's
usual time), meshes a body coarse while plans outrun builds (`moving`,
`coarse`) and finely 300 ms after they stop, builds an open task's body only
up to the edited feature (`QueuedBuild::up_to_edited`, the rest once
settled, `preview_rest`), and builds bodies on a pool of 2 to 4 threads
(`build_loop`) beside the worker thread for everything else.
`examples/rebuild_bench.rs` (`--ops`, `--before`) measures it.

Import performance: the per-solid work and each mesh's face pass go through
`ogeom_core::parallel::map_ordered` (order-preserving, so output is identical
at any thread count, which `tests/kernel/step_import.rs` asserts). Never nest two
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
  `app/commands.rs::apply_ui_commands` (two-phase dispatch preserves ordering).
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
`PRINTCAD_BENCH_ORBIT` are bench hooks (`PRINTCAD_EDGE_MIN_PX` /
`PRINTCAD_NO_EDGES` are the renderer's, mesh.rs); `PRINTCAD_BENCH_SKETCH=1` opens a
constrained sketch for editing and `=pad` pads it and opens the Pad task;
`PRINTCAD_BENCH_EDIT=<feature name>` opens that feature for editing (a
sketch's session) once the kernel has been idle a moment, as a double
click on its row would;
`PRINTCAD_BENCH_POINTER=<fx>,<fy>` then holds the pointer at that
fraction of the viewport, with `PRINTCAD_BENCH_TOOL`'s tool in hand, so
a capture shows a tool's cues;
`PRINTCAD_BENCH_SELECT=<n or name>` selects a body once it has geometry,
the way a click on its tree row would, so a capture shows the selection
overlay (with no body of that name, the feature named exactly so, which
moves its body's tip as a click does); `PRINTCAD_BENCH_ISOLATE=<name>` isolates the view to the first
body whose name holds the text once it has geometry (the view mode
Isolate is); `PRINTCAD_BENCH_CHAT=<message>` sends the message, its `@` mentions
taken as picks would, in a chat with no agent behind it once the kernel
is idle (after `PRINTCAD_BENCH_SELECT`), and opens the assistant on it
with `@` typed, so a capture shows chips and the mention list;
`PRINTCAD_BENCH_TASK=appearance|placement|history|texture|print_layout|array|note|mark`
opens that application task on the first body (`note` its first note, or
a new one); `PRINTCAD_BENCH_NOTE=<number>` arms the note tool with that note's card
open; `PRINTCAD_BENCH_PREFS=<page label>`
opens Preferences on that page once; `PRINTCAD_BENCH_CLICK=<fx>,<fy>` snaps to a corner view and makes
one selection click at that fraction of the viewport, logging what the
pick, the edge test and the face hover saw and what got selected, and with
`PRINTCAD_BENCH_TOOL=<tool id>` then runs that tool on the selection as a
toolbar click would and logs every feature's rebuild error (frame.rs);
`PRINTCAD_BENCH_SECTION=<aim>[@<offset>][!][^<way>][~<body>]` turns the clipping plane on
once there is geometry (after the click hook's click), aimed `x`, `y`,
`z`, `view`, `face` (the selected flat face, else waiting for a click) or
`points:x,y,z;x,y,z;x,y,z` or `saved:<name>`, moved along its normal, `!` flipped, `^0`/`^1` a
wedge's second plane, the body named after `~` left uncut;
`PRINTCAD_BENCH_UI=<steps>` plays pointer steps into the window once
the scene has settled (`ui/bench_ui.rs`: `click:x,y`, `right:x,y`,
`move:x,y`, `wait:n`, `;`-separated, logical points; `view:fx,fy` a
click at that fraction of the viewport, handed to the active bench as
its clicks are, `type:text` typed into the field with the keys, `key:name`
a key, `Shift+D` with its modifiers, `press:x,y`/`release:x,y` the
primary button held and let go, the `move`s between them a drag), and a
picture
waits for them, so a capture shows a panel in a state only clicks reach;
`PRINTCAD_BENCH_PICTURE=<path>` writes one picture of the view there
once the scene settles (the renderer's scene readback, `request_capture`),
and `PRINTCAD_BENCH_WINDOW=<path>` one of the whole window, panels and
menus with it (`CaptureKind::Window`);
`PRINTCAD_AUTOSAVE_SECS` shortens the autosave wait (`app/recovery.rs`:
copies of edited tabs in the data folder's `recovery/`, taken away when
saved, closed or on exit, offered on the start page after a crash);
`PRINTCAD_BENCH_WORKBENCH=<bench id>` switches to that bench once the
document has a body (so `PRINTCAD_BENCH_TOOL` reaches its tools);
`PRINTCAD_WIDGET_MAP=<path>` writes where the toolbar's buttons and
groups are (`ui/widget_map.rs`: `button:<tool or command id>`,
`group:<group id>`, physical pixels from the window's corner) whenever
they move, so a scripted check clicks by id;
`PRINTCAD_BENCH_REPAIR=1` asks for the repair of every broken body once,
`PRINTCAD_BENCH_CONVERT=1` the conversion of every mesh body. Every
`PRINTCAD_BENCH_*`, `PRINTCAD_OPEN_*` and `PRINTCAD_EXIT_AFTER_MS` hook is
read once at start into `app/bench_hooks.rs`'s `BenchHooks` and played by
`drive_bench_hooks`; any of them skips the start page and leaves the
user's kept session alone. The 1 s `printcad.frame` log reports
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
(`render_wgpu/src/core.rs`) changes; every frame copies that image under the
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
`on_command`) · RMB held still (300 ms) = the radial menu (`app/radial.rs`,
`ui/radial.rs`: the active bench's slices round the press, the user's
`UserSettings::radial` or the bench's `WorkbenchDescriptor::radial` (a
package's `Registration::radial`) or its most used tools by
`UserSettings::tool_uses`; letting go over one starts the tool as its
button would or runs the command; Preferences › Input › Radial menu
edits them) · wheel = zoom · LMB = select (click sketch → tree-select; click
solid → face-first, double click → the whole body the face belongs to, one
part of an assembly, and in Design the tree opens to its row; LMB drag
in sketch = box select; ctrl = additive: Ctrl+click adds or removes a
face of the selected body (`session.earlier_faces` beside the last,
`last_face_hit`; `face_pick::toggled`), and an edge, faces and edges mixing;
benches see every picked face as `ctx.selected_faces` /
`selected_faces_in(body)`, `selected_face` the last; the status bar says
"3 faces, 2 edges"). The
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
disabled; pan/zoom/roll allowed). The tree reads as the body's history in
order: selecting a feature moves the body's tip to it (off it, when it is
the last; `move_in_time_to`), later features draw muted and out of the
solid, and a feature made then goes in right after the tip, which follows
it (`Document::insert_at_tip`, a seq swap per step, like Move up). A tip
move on its own is navigation: `OpJournal` makes no undo step of a gesture
that only moves tips. In the tree, bodies and features start
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

- Each crate's integration tests are one program (`tests/<name>/main.rs`
  with a module per file: `kernel_ogeom/tests/kernel/`,
  `core_document/tests/document/`, `wb_assembly/tests/assembly/`,
  `wb_sketch/tests/sketcher/`): every program links the crate and its
  dependencies again, which costs far more than running tests. Add a test
  file as a module there, never as a new file beside `main.rs`'s
  directory. Dependencies build with line tables only
  (`[profile.dev.package."*"]`) for the same reason.
  Our own crates build with line tables too (`[profile.dev] debug =
  "line-tables-only"`, about a fifth off a test build); a debugger session
  sets `CARGO_PROFILE_DEV_DEBUG=full`.
- The suite has a time budget (`scripts/test-budget.mjs`: 60 s a program,
  150 s in all, on CI, two programs running at a time); a test that needs
  a big model is `#[ignore]` with the reason, run on request. While
  working, test what a change reaches: `node scripts/test-budget.mjs
  --changed` (`scripts/affected-crates.mjs` against the merge base with
  origin/master: changed crates and everything depending on them,
  dev-dependencies included; the changed crates' programs run first).
- CI (`.github/workflows/ci.yml`): a push to master builds, lints and
  tests everything on Linux, Windows and macOS. A pull request lints
  everything on all three, but tests on Linux only what it reaches
  (`scripts/ci-scope.mjs`), and on Windows and macOS only when it touches
  code that differs by platform (the system-facing crates and files, any
  file with a `cfg(windows)`/`cfg(unix)`/`target_os` gate, the manifests)
  or carries the `platforms` label. Changes to CI, the test scripts or the
  manifests test everything.
- Tests stay a bounded share of the code: `scripts/test-lines.mjs` counts
  test lines per crate and fails CI above its ceiling. A new case goes
  into an existing table or shared harness (the sketcher `Harness`,
  `tests/kernel/common.rs`) rather than a near-copy of a sibling test; a
  test whose property a stronger test already asserts is retired. A
  behaviour has one owner at its strongest boundary: a tool proved
  through the viewport's clicks is not proved again by unit clicks on its
  state, and a slow geometric check shares its builds (a `OnceLock`)
  and meshes no finer than its tolerance needs.

- Sketcher end-to-end tests drive `on_input` with real viewport-pixel clicks:
  `wb_sketch/tests/sketcher/interaction.rs` (reuse its `Harness`).
- Full-stack sketch→feature→solid pipelines:
  `kernel_ogeom/tests/kernel/design_stack.rs` (dev-deps on wb_design/wb_sketch).
- Solver/geometry math is unit-tested next to the code. Assert geometric
  properties (bounds, tangency, closure), not implementation details.
- Before committing: fmt, clippy (zero warnings), full test suite, and a
  short `cargo run` smoke check watching for `printcad.gpu` output. For
  UI work, a headless capture of the release build (`PRINTCAD_BENCH_SKETCH=pad
  PRINTCAD_EXIT_AFTER_MS=…`, `grim`, `ydotool` for keys) is the smoke
  check; verify on a small STEP, never the huge assembly files.
- A change people can see (a panel, the view, a solid's shape) shows it in
  its pull request: a picture before and after, taken the same way from
  the same document and view (`PRINTCAD_BENCH_PICTURE=<path>` with the
  bench hooks that set the scene up, e.g. `PRINTCAD_BENCH_SKETCH=pad`;
  `PRINTCAD_BENCH_WINDOW=<path>` for a panel or menu), or
  for a solid its measures before and after (`project-tool check`, the
  kernel's volume). Pictures go in the pull request, never the repository.
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
  (`doc_render::face_pick::face_submesh`). A mesh without faces (a sketch, a datum, a
  document saved before they were recorded) falls back to the plane through
  the hit. References find their faces by name (`kernel_api::naming`):
  the chain names every face as it builds (`kernel_ogeom::naming`: a
  feature's walls after the sketch elements they were swept from, which
  the sketcher's profiles carry as `ProfileWire::names`; its ends after
  where they stand; other new faces by surface kind and facing; a face a
  later op splits, trims or merges after the faces the kernel's history
  says it came from (every `Built.history` an op's kernel calls return
  is recorded, `naming::record`, and followed back), a face generated
  from an edge (a fillet's round) after that edge's two faces, and where
  the history says nothing after the faces it lies on or covers; the
  faces an op left alone recognised by a `Print`), each op under its
  feature's name (`ChainRequest::tags`, the worker's tags). The
  mesh carries `face_names` and `edge_faces`; a pick keeps them
  (`FaceRef::name`, `EdgeRef::faces`, `FacePick`, `EdgePick`, datum
  anchors, `FaceSupport` of a sketch placed on a face), and a rebuild finds
  a named face or edge by name first (`naming::find_face`/`find_edge`,
  reading the running solid's names the chain sets for the op), falling
  back to the stored point + normal. A reference whose names are lost is
  found by position only within a tenth of the solid's diagonal
  (`probe::position_reach`, else a `ChainError` saying it is gone), and
  the op that took it is warned (`SolidBuildResult::warnings`, kept as
  the derived `FeatureNode::warning`, the row's Warning chip and the
  scripts' `warning`); a probe's or projection's warning
  (`probe_warnings`, `projection_warnings`, by their place in the list)
  goes to the feature that asked it, a datum or a sketch. Edges picked on
  another body (a surface step's `CurveSource::BodyEdge`, a pipe's
  `EdgesOf`) are found by position, `dressup::picked_edges_elsewhere`,
  since the chain's names are its own body's. A sketch placed on its body's face
  follows it: Design's plan asks where the face stands at the
  sketch's place in history, and `Workbench::derive_on_solid` moves the
  plane from the placement it recorded. One on a face a borrow lends
  (`FaceSupport::lent_by`) is answered from the borrow instead
  (`answer_lent_faces`, which finds the face in the lender's mesh by
  name); `SketchAttachRequest::face_origin` says which a face is, and a
  face of another body is not followed at all. `TriMesh.edge_ids` names the kernel edge of every
  outline segment (the tessellator draws the outline from the kernel's own
  edges, each to the chord the faces agreed on; triangle boundaries are only
  the fallback), so a click within a few pixels of an outline picks the
  whole edge (`app/edges.rs`: hover, Ctrl-additive selection, highlight line
  bodies, the hover card's edge length, the measure tool's snap). Benches
  see picked edges as `ctx.selected_edges` (point, direction, length,
  faces); Design's fillet and chamfer store them as `EdgeSel::Edges`
  picks the kernel resolves through `EdgeSelection::Picked`.
- "Through all" derives its length from the base solid's bounding box.
  Up to face (`ExtrudeTermination::UpToFace`, the base's face nearest the
  pick), to first and to last trim a long prism by the half-space of the
  target face's whole surface, pushed out by the offset along an offset
  surface, so flat and curved targets alike stop exactly on the face.
- Helix: the height + turns + growth mode sends `SweepKind::Helix` its
  turns and growth per turn (in place of the cone angle); at height 0 it
  is a flat spiral. A subtractive helix with "keep inside" is
  `BooleanOp::Common` (a pattern cannot repeat it).
- Pipe: `SolidOp::Pipe` carries a `PipeFrame` (rotation-minimizing,
  Frenet, auxiliary path, binormal; files with the old `frenet` flag read
  as the first two), a `PipeCorner` and further `sections`; an open path
  runs from the end its profile sits by (`sketch_spine`). `pipe_tool`
  (`ops/loft_pipe.rs`) hands the frame to the kernel's `PipeLaw` and the
  corner to `PipeCorners` (`make_pipe_shell_with`), and extra sections to
  `make_pipe_sections`, the profile placed at the path's start and each
  section where it stands. A revolution up to a face whose plane holds its
  axis stops at one angle; any other face goes to `make_revolution_until`.
  A thickness's arc join is the kernel's `Join::Arc`.
- Hole threads: `wb_design/src/hole_tables.rs` holds the thread standards
  (ISO metric coarse and fine, UNC/UNF/UNEF, BSW/BSF, BSP G and Rc, NPT),
  their classes, the ISO 4762/10642 seats and the user's `hole_cuts.json`
  profiles (read once at start, copied into a hole when picked, so geometry
  never depends on the machine). A hole's `thread` is a `ThreadSpec`
  (standard, designation, class, hand); `metric_index` in old files
  deserializes into it. A threaded standard hole drills its tap diameter
  (a taper thread its minor diameter at the face, narrowing 1:16),
  clearances otherwise; "Modeled thread" also cuts the standard's groove
  (60° or 55°) along a helix out to the major diameter, along a cone for a
  taper (`thread_cut` in `wb_design/src/build/holes.rs`). A cosmetic thread
  (`cosmetic_thread`) draws a helix instead, and its designation reaches
  the host through `Workbench::cosmetic_threads` (`ThreadCallout`, kept by
  `DocumentService::evaluate` as derived `Document::cosmetic_threads`),
  which the parts list's Threads column, `app/annotations.rs`'s callouts
  and a STEP export's callouts (`export.rs::body_notes`) read. Drill points and
  counterdrill cones are kernel cone primitives cut at each centre.
  `docs/HOLES.md` is the user guide.
