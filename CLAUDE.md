# printCAD: agent notes

Linux-native parametric CAD app aimed at FDM/SLA printing.
Rust workspace + wgpu (Vulkan, Metal, DirectX 12) + egui + the pure-Rust ogeom B-rep kernel
(crates.io, `Cargo.lock` holds the exact version).

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
cargo test --workspace            # full suite (~1500 tests)
node scripts/test-budget.mjs      # the same, timed against its budget (what CI runs)
node scripts/test-budget.mjs $(node scripts/affected-crates.mjs)   # only what a change reaches
cargo clippy --workspace --all-targets   # CI enforces -D warnings
cargo fmt --all                   # CI enforces --check
```

- The document server is a separate binary the app spawns from its own
  directory, so a release run needs `cargo build --release` for the whole
  workspace: `-p app_shell` alone leaves `target/release/printcad-serverd`
  missing and the app falls back to direct file I/O with a warning.
- No system CAD libraries needed: the ogeom kernel is pure Rust, released to
  crates.io. Bump it with `cargo update -p ogeom`; a commented line in the
  workspace `Cargo.toml`'s `[patch.crates-io]` points at a local checkout
  for kernel dev.
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
  for the list). Network work and compiling
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
  `RELEASE_NOTES.md`, and a tag `vX.Y.Z`: `.github/workflows/release.yml`
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
  The CI's `platforms` job runs clippy and the tests on Windows and macOS.
- The app also builds for a browser page: `scripts/build-web.sh` (the
  release build for `wasm32-unknown-unknown`, wasm-bindgen at the version
  `Cargo.lock` pins, `wasm-opt`; `web/index.html` starts it, showing the
  module's download) writes `web/dist`, and a second build with atomics and
  shared memory into `threads/` (the standard library rebuilt,
  `RUSTC_BOOTSTRAP`; `WEB_THREADS=0` skips it), which the page loads when
  cross-origin isolated (`third_party/coi-serviceworker` gives a static host
  the headers): each kernel worker then starts a `wasm-bindgen-rayon` pool
  and lends it to the kernel (`kernel_ogeom::threads::lend_rayon`,
  `parallel::set_pool`). A release builds it (`release.yml`'s `web` job,
  `printcad-web-X.Y.Z.tar.gz`) and `pages.yml` puts the latest release's
  under the site's `app/`; CI clippies the browser target. Everything that differs sits behind `cfg(target_arch =
  "wasm32")`, so a desktop build compiles exactly what it did:
  `app_shell/src/platform.rs` (`spawn` runs work at once on the page,
  `read`/`write` are the picked files held in memory and downloads, and
  under `/printcad` the files the page keeps (documents, the recent list,
  autosaved copies; IndexedDB, read back before the app starts), `remove`,
  `list`, `exists`, `kept_dir`, `ON_PAGE` and `offers` (the commands a page
  leaves out of its menus, palette and keys), `set_unsaved` (the page asks
  before it is left), `confirm`, `ask_name`, `temp_dir`, `scratch_file`; `platform/web.rs` the
  page's picker and downloads), `app/server.rs` (`BrowserFiles` in place
  of the daemon), the kernel's jobs in the page's workers
  (`kernel_pool.rs`, `web/kernel-worker.js`: one for requests, more for
  builds, MessagePack between them, an import's bodies sent a chunk at a
  time and its source left with the page, which holds it; Cancel ends the
  busy one, a worker that dies has its job answered with why (its panic
  told first) and is started again, as the request worker is after every
  import, a WebAssembly memory never shrinking), the renderer awaited (`Renderer::initialize_async`; WebGPU,
  else WebGL2), settings in the page's storage, `kernel_ogeom::files` (the
  reader imports go through), and `web_time` for every clock (the
  standard one panics on that target). The console's Lua is wasmoon
  (`third_party/wasmoon`) in a worker of its own (`web/lua-worker.js`,
  `scripting/src/thread_web.rs`, `web_prelude.lua`: JSON both ways, a
  script awaiting each command's answer, Stop ending the worker and its
  globals). The 6-DoF mouse comes through WebHID (`app/sixdof/web.rs`:
  devices granted before open at start, Preferences › Input › 6-DoF mouse
  opens the browser's chooser, each report decoded by
  `sixdof::hid::Decoder`). Packages run through `wb_wasm`'s `web.rs` with
  jco (`third_party/jco`, `scripts/vendor-jco.sh`); the page keeps each
  installed archive in IndexedDB (`app/packages_web.rs`, a `Package` held
  in memory, `Package::from_archive`), installs from a file or a store's
  `mirror` (the registry's own copy, since a release's download refuses a
  page; `store::listed_package`), and has no GitHub installs or update
  checks. Agents' sockets, the document daemon and the command line are
  desktop-only. `egui-winit` is
  patched (`third_party/egui-winit/PATCHED.md`) until a release builds for
  the browser.
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
  `wb_wasm` test holds equal to `bench_api`'s.
- `workbenches/wb_wasm`: workbench packages (`docs/PLUGINS.md`, RFC 0001).
  One wasmtime engine (`engine.rs`: an epoch ticker that runs only while a
  call or job does, 25 ms for frame and input calls, 1 s otherwise; the
  compiled component cached beside `bench.wasm`), `exports.rs` (the
  guest's exports as the `Exports` trait, whichever runtime holds the
  instance, `Budget`, `Fault`), `host.rs` (`Reach`, the host's side of an
  instance: what the call in progress may reach, `Access::None/Read/Write`,
  a raw pointer valid for that one synchronous call; the `doc.*` calls a
  package makes on its own kinds only, recorded like any edit), `guest.rs`
  (wasmtime's bindings and store; instantiate with WASI, the package's
  `data/` preopened as `/data`, the network only when granted, a memory
  cap; a trap or overrun replaces the instance, three turn the bench off),
  `web.rs` (the same on a browser page: jco transpiles the component in
  `web/package-worker.js`, the instance runs on the page and is called as
  wasmtime's, with no budget, no `/data` and no network; `web/jobs.rs` a
  job in a worker of its own, stopped by ending it), `jobs.rs` (a job in its own
  instance on its own thread, progress and cancel, native helpers under
  the `helper` grant), `bench.rs` (`WasmWorkbench`: the `Workbench` trait
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
  `tests/rogue` (misbehaves on request) and `tests/probe` (a sketch's
  profile, a cancelled job, a table cell, a formula, the save dialog:
  what the gear leaves out); `wb_wasm/tests/packages.rs`
  builds them with cargo and runs them through the host.
- `kernel_api`: pure data contract (TriMesh with per-triangle kernel face ids, ProfileWire w/ ellipse+B-spline
  segments, `SolidOp` = sweep/loft/pipe/primitive/dress-up/transform/boolean,
  ExtrudeTermination, TessellationSettings, ChainError). No geometry code.
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
  instancing. Tests marked `#[ignore]` document kernel-side gaps; grep for
  `kernel:` in `tests/` before assuming a feature is wired wrong.
- `core_document`: Document (feature tree DAG, bodies w/ `tip`, tar `.prtcad`
  persistence), `Workbench` trait + runtime context,
  workbench registry (`service.rs`), core datums (`datum.rs`:
  plane/line/point/coordinate system + attachment + offset, shared across workbenches;
  an attachment on its own body's solid (a face, edge or point picked, a
  circle's centre, the centre of mass and axes of inertia) keeps its picks
  as `ShapeProbe`s, which Design's plan asks of the solid where the
  datum stands in the history (`BuildPlan::probes`, answered by
  `execute_solid_chain_probing`); the answers are derived state
  (`Document::store_probe_answers`) that `datum::derive` folds into the
  datum's values, so it and what is built on it follow the solid).
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
  note card, the button set, `tool_button`, `section_header`, `QtyField`,
  `select_field`, `PrefRow` + `pref_group`, `planned`,
  `completion::completing_text_edit` (a text edit whose dropdown offers
  the five names nearest the word at the cursor, arrows, Enter/Tab and
  clicks, fed by `core_document::formula_candidates` wherever a formula
  is typed), and `Tab` +
  `tab_plus`, the one tab every strip draws: documents, chats, the property
  panel's pages, Preferences), `icon` (the SVG
  set under `icons/`, vendored by `scripts/vendor-icons.mjs` into a
  generated `icon_table.rs`; `icon::texture/draw` rasterize with a
  font-free usvg, since the system-font scan is far too slow for 200 icons;
  `select.svg`, `expression.svg` and `chevrons-down.svg` are
  hand-authored locals). The same
  table carries the `motion-*` drawings (200×200, their own colours, one
  per movement of a 6-DoF mouse), drawn through `icon::drawing`, which
  rasterizes for the size it is shown at rather than the icon size. A test
  fails when the table and the directory disagree. `markdown` draws the
  chat's messages: `pulldown-cmark` into a small block tree, each run of
  text one selectable label whose links are found from where a click lands
  in the laid-out text, code blocks with a copy button.
- `workbenches/wb_sketch`: sketcher: `tools.rs` + `tools/{draw,modify,
  transform}.rs` (state machine), `geom2d.rs` (intersection/sampling math),
  `snap.rs`, `solver.rs` (LM, uniform constraint records + diagnostics),
  the line tool's drag from its end (`LinePress`, `step::line_arc_click`:
  the polyline's tangent arc, a `sketch.draw` point with `arc = true`),
  reference pictures (`images.rs`, `Sketch::images`, the file a document
  asset, decoded once each, shown through `Workbench::get_screen_space_images`
  as egui textured quads under the lines; `sketch.image`, `sketch.set_image`,
  File › Import for PNG and JPEG),
  the profile's regions shaded while editing (`get_overlay_meshes` over
  `KernelQueries::profile_mesh`, cached by profile) and out of it (the
  passive geometry's `PassiveRegion`, which the host meshes once per
  revision and draws see-through behind the lines), loose ends ringed,
  `profile.rs` (closed-wire extraction), `overlay.rs` (screen-space rendering
  while editing), `glyphs.rs` (constraint icons and dimension layouts),
  `constrain.rs` (which constraint a toolbar action creates for the
  selection's shape), `panel.rs` (the task panel), `style.rs` (icons and
  names per element/constraint kind).
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
  `app/textures.rs` resolves faces and pictures (`Pressing`), makes the
  view's preview on threads (`drive_texture_previews`, the last one shown
  while a newer is made) and hands export a `finish` per body
  (`ExportBody::finish`, applied in the body's frame before placement);
  `ui/texture_task.rs` is the task. `docs/TEXTURES.md` is the guide.
- `workbenches` facade: `register_all_workbenches` (built-in benches) and
  `register_packages` (installed packages, after them, as the user allowed;
  `PackageStatus` for the Preferences page).
- `workbenches/fixtures` (`bench_fixtures`): ready-made scenes built from
  the benches' feature types (a dimensioned sketch, padded, pocketed) for
  the app's `PRINTCAD_BENCH_SKETCH` hook and tests; the host composes
  benches only through it, never by naming them. `app/seam_lint.rs` fails
  when a bench crate, id or feature type appears in `app_shell/src`, and CI
  greps for the same.
- `workbenches/wb_design`: Pad/Pocket/Revolution/Groove/Loft/Pipe/Helix/
  Primitive/Hole/Fillet/Chamfer/Draft/Thickness/patterns/Boolean features
  (`feature.rs`; every one that fuses or cuts carries `refine`, which the
  build follows with a `SolidOp::Refine` merging coplanar faces; the
  Preferences switch is only the default for new features, so geometry
  never depends on who rebuilds it), per-feature panel editors (`editors.rs`), the task
  lifecycle (`task.rs`: snapshot on open, live edits, Cancel restores or
  deletes a tool-created feature); `build.rs` translates a body's feature
  history into kernel `SolidOp` chains (`BuildPlan` maps op index → feature
  for error attribution).
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
  operations. A kind the kernel cannot build carries `waits` (tool
  `planned`, no command), with an ignored test per gap in
  `tests/kernel/surface_ops.rs`.
  Commands `surface.*` take `sketches`, `body` (a body or a feature in it)
  and any field by name, refusing a step that misses its inputs
  (`SurfaceFeature::missing`); `surface.set` changes a step's fields and
  curves, `surface.check` returns a body's joins. Check continuity (`surface.check`) asks
  `KernelQueries::continuity` (the kernel's `analyse_blend` over every
  face: gap, crease and curvature jump per shared edge, G2 below
  `G2_PER_MM`) of the selected body and labels the edges through
  `get_screen_space_labels`. The curvature map and zebra stripes
  (`analysis.rs`) paint the selected body as one `get_overlay_meshes`
  mesh, cached by the body's geometry revision: the map's values from
  `KernelQueries::curvature` at the mesh's vertices, the stripes from its
  normals, each triangle split along the stripes' borders; colours from
  the palette's `analysis_*` and `stripe_*`.
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
  and the pull halved back to contact; the release records `asm.place`),
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
  joint settings and moving a body by numbers (`panel.rs`: every panel is
  declared widgets, `task_widgets`, drawn by `core_document::panel` as a
  package's is, its events handled in `task_event`, which edits through
  the bench's own commands, `asm.set`, `asm.place` and the rest, where
  one makes the change; `panel_tests.rs` checks each against its
  command). Solves run inside the
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
  or picture, mapped once its frame is done); per-body mesh cache keyed by
  (id, revision). Edges are instanced screen-space quads (`vs_edge`), any
  width on any backend; the clipping plane discards per fragment. The pick
  pass writes depth into a colour target as well, since a depth texture
  cannot be copied in part. The window takes a plain format, as egui
  blends for; the scene draws into an sRGB texture that the blit encodes
  onto it.
  `FrameSubmission.grids` (`GridSubmission`, `grid.rs`): a patch of any
  plane whose lines `grid.wgsl` works out per pixel, three decades at once,
  each faded by its spacing on screen, depth-tested and never picked.
  `app/scene_guides.rs` lays out the ground grid and origin planes; View ›
  Grid and Origin planes turn them on (both start off), and an edit
  session holding the view hides them. While the active bench's
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
  `with_tab` turn must not touch it. `app/` modules: `frame.rs` (per-frame loop),
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
  `sixdof.rs` (6-DoF mouse reader thread; holds the puck's current deflection,
  which `camera::apply_device_motion` integrates once per frame).
  `ui/` is one module per region: `menu_bar`, `toolbar` (groups: the
  standard runs, Scripts, the bench switcher and each run of a bench
  category along `ToolDescriptor.row`; each dragged by its grip along a
  row, to another or to a new one, kept in `UserSettings.toolbars`, a
  group it does not list placed where its bench puts it; variant
  dropdowns), `combo_view` (tree +
  `property_panel`), `feature_tree`, `task_panel` (host of the workbench
  task; OK/Cancel/Enter/Esc), `status_bar` (ending in the log, console
  and assistant switches), `view_toolbar` (floating
  pill; in perspective it carries the field of view, dragged or typed,
  which `CameraController::set_field_of_view` changes keeping the framing), `hud` (workbench HUD corners, OVP card, hover card), `overlays`
  (line/mark/label painters), `start_page`, `preferences` (modal on a
  draft `UserSettings`, committed by `CommitSettings`), `command_palette`
  (Ctrl+K), `step_import_modal`, `log_view`, `host_ctx`.
  Every menu's rows are egui buttons (hover and disabled states come
  with them), inside `ui_kit::widgets::fitted_menu`, which scrolls what
  would run past the window's bottom. The scene hovers and picks only
  while the pointer is on it, not on a menu or card over it. An edit
  session on a plane opening clears the view's selection.
  Row menus (`feature_tree.rs`, `context_menu.rs`, the shared
  `body_menu.rs`) are flat; what needs numbers or choices is an
  application task (`host_tasks.rs`: Placement, Appearance, History),
  held on `UiLayer` for its tab and drawn in the task panel while no bench
  task is open, editing the document live as a bench task does and
  closing through `TaskClosed` plus the `Recorded` calls it would make.
  Closing a tab, the window or the app with unsaved edits asks in the
  window (`app/unsaved.rs`, `ui/unsaved_modal.rs`) and the close waits for
  the answer; quitting asks for each tab with edits in turn.

The `Workbench` trait is the only seam between the host and a bench; the
host never names a bench: `app/seam_lint.rs` and a CI grep over
`crates/app_shell/src` (the lint's own token list excluded) refuse any
line that does. `descriptor()` says
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
`viewport_hud()`, `status_items()`, `editing_feature()`,
`get_screen_space_overlays/polygons/marks/labels()` feed the viewport and
chrome (polygons are filled fans drawn beneath the lines: the Assembly's
  Move handles, `wb_assembly/src/handles.rs` over `transform_gizmo`);
`WorkbenchRuntimeContext::pixels_per_point` carries the UI scale: input and
overlay coordinates are physical pixels, while the transform gizmo's adapter
converts them to and from logical pixels. `cancel_pointer_gesture()` restores
a held pointer gesture when the window loses focus, before UI event filtering;
`ui_settings()` draws the bench's Preferences page (a rail entry for
each bench whose `has_settings` is true; a package's is true when its
settings page has widgets); `feature_info`/`passive_geometry`/`pick_feature`/
`delete_feature`/`property_hints` answer for the feature kinds a bench
claims; `linked_features` names features of other bodies the tree lists
under a body too (a joint under the body it holds to); `derive_on_geometry`
brings a feature's working data up to the bodies' geometry during
evaluation (an assembly joint re-finds its named faces on a rebuilt body,
and `values_moved` re-solves); `faded_bodies`
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
counts), never dragged, and projected again. New projections take the
Construction mode (a guide in it, counting out of it; the commands' `counts`
flag, off unless given); projected ends at one spot join in the profile
once per editing session (in place when the curve is the same kind). Its
intersection variant takes picked faces instead and adds the curves where
each crosses the sketch plane (`KernelQueries::section_face`, the face's
solid sectioned by the plane's half space and the edges on the face kept;
`ExternalSource::section`), refreshed the same way. File › Export
(`app/export.rs` over `kernel_ogeom::export`) writes the visible or the
selected bodies as STEP, or as STL or 3MF meshed afresh at the dialog's
tolerance and welded closed, on a thread of its own; the start page's
Export for printing walks it on the pocketed example. File › Send to
slicer writes the visible bodies to `$TMPDIR/printcad/slicer/` in the
format of `PrintingSettings::slicer_format` and runs
`slicer_command` on it (`{file}` places the path, else it goes last;
empty uses `xdg-open`). File › Print layout (`app/print_layout.rs`, a
task) lays each part on its largest face it stands on and packs the
copies on the bed, plate after plate, without moving the bodies; the
counts come from `Workbench::print_parts` (the Assembly's parts list),
and export, the slicer and `doc.print_layout` take the layout. The
parts list's print counts, volume and filament mass are in
`docs/PRINTING.md`, with the Hole's nut trap. A save carries a
CPU-rendered preview (`thumbnail.rs`, in the save worker) as the
container's first entry, `thumbnail.png`, which
`Document::read_thumbnail` reads without unpacking the rest; the recent
cards show it. What's new reads `crates/app_shell/RELEASE_NOTES.md`, and
a test fails when the running version has no entry there. The Edit menu's Cut/Copy/Paste go to the
active bench as `MenuScope::EditMenu` commands (the sketcher keeps a
geometry clipboard); the view toolbar's clipping plane (`camera/section.rs`, per tab, a
plane square to X, Y or Z), or while the active bench returns one the
plane of any direction `Workbench::clip_plane` gives (the sketcher's
per-sketch section view), reaches the renderer as
`FrameSubmission.clip_plane`: every scene shader and the pick pass write
a clip distance, the cut's back faces draw as a flat darker section, and
CPU edge picking skips what it hides; the toolbar's Measure arms a readout
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
source still has the snapshot it was made from).

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
(`OgeomKernel::read_solid`: a STEP/IGES file's first body, a mesh file
converted) into the base, invalidating the body, or into its shape.
Design's Delete Faces (`DesignFeature::DeleteFaces`, `SolidOp::RemoveFaces`)
takes picked faces away through the kernel's `remove_faces`, the
neighbours closing the openings; Offset Faces and Move Faces
(`SolidOp::OffsetFaces`/`MoveFaces`, a distance, or a translation and a
turn about an axis as a rigid row-major matrix) go through its
`offset_faces`/`move_faces`, the neighbours following on their own
surfaces. Recognize holes (`design.recognize_holes`,
`wb_design/src/recognize.rs`) asks `KernelQueries::recognize_holes`
(`kernel_ogeom/src/holes.rs`: concave cylinder faces grouped by axis and
radius into whole bores, each end an opening, a flat bottom or a coaxial
cone, from the faces sharing its edges) and adds one Delete Faces for
every hole's faces, then per set of alike holes a hidden sketch of their
centres on the opening plane and a Hole feature.

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
(`PRINTCAD_WRITE_DOCS=1` rewrites it). A `CommandSpec` carries `notes`,
`examples` (Lua ending in `assert`s) and `see_also` (`.note`, `.example`,
`.see_also`; a package's `bench_api::Command` the same), which the
reference prints; `every_command_example_and_recipe_runs`
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
drops on X11 only). The `view` tool and `doc.picture` (headless too)
draw through `proof.rs`: a `Request` (view by name, angles or direction
in the axis preset's frame, bodies, highlight, markers, section, edges,
xray, size, annotate) over `thumbnail::draw` and its `Look`, framed to
what is drawn, labels as SVG through usvg with only the bundled Plex
face (`ui_kit::theme::SANS_REGULAR_TTF`); every view but `current`
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
FormulaField` over `core_document::DocumentFormulas` (the document's last
values) in the property panel's Parameters group, Design's and the
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
`TaskClosed` closes the gesture. A `TaskInfo::stepwise` task (the
sketcher's editing session) is the exception: each edit in it is an undo
step of its own.

**Feature preview.** While a bench's `editing_feature` is one that builds
solid (Design's open task), `recompute.rs` asks its body's builds for a
preview (`request_build_solid`'s `preview`, `execute_solid_chain_previewing`
over the feature's op range): the chain keeps the feature's tool and the
body before it (an adding feature) or after it (a cutting one) in
`SolidBuildResult::preview`. The body's geometry is then that solid, so
picks match what is drawn; `session.previews` keeps the whole solid, put
back when the task closes (and in the save snapshot), and the tool, drawn
`front_only` in `rendering.preview_color`/`preview_opacity` with its edges
over its faces (the renderer's `TranslucentFront` pass, back faces culled).

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
(`session.builds_in_flight`): a plan made while it builds waits in its
place, a newer one replacing it, and goes when the build lands
(`build_landed`), so a drag that changes a feature every frame builds only
the latest shape rather than replaying each step. A history that changed shape goes through
`invalidate_body`; a history jump or Recompute All through `invalidate_all`.
Builds skip what an edit did not change (what is left, the ops themselves, is in `docs/ROADMAP.md`):
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
4. When the fix lands: bump the ogeom rev pin in the workspace `Cargo.toml`,
   un-ignore the matching tests, rerun the full suite.

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
  (`profile::loose_ends` names what was left out).
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
- Serde compatibility: new fields on persisted types (features, sketch) take
  `#[serde(default)]` so old `.prtcad` files keep loading.
- A `.prtcad`'s meshes are entries of their own (`mesh/<uuid>.bin`,
  MessagePack; `ImportedGeometry::mesh_path`), the JSON carrying an empty
  mesh in their place (`MESHES_APART` while it is written), since a large
  assembly's meshes as JSON text passed what one buffer may hold on a
  32-bit target; a document whose JSON holds them still loads. A plain
  `.prtcad` is an uncompressed tar on a desktop; a page saves zstd, and
  both read either by its first bytes.
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
overlay; `PRINTCAD_BENCH_TASK=appearance|placement|history|print_layout` opens that
application task on the first body; `PRINTCAD_BENCH_PREFS=<page label>`
opens Preferences on that page once; `PRINTCAD_BENCH_CLICK=<fx>,<fy>` snaps to a corner view and makes
one selection click at that fraction of the viewport, logging what the
pick, the edge test and the face hover saw and what got selected, and with
`PRINTCAD_BENCH_TOOL=<tool id>` then runs that tool on the selection as a
toolbar click would and logs every feature's rebuild error (frame.rs);
`PRINTCAD_BENCH_PICTURE=<path>` writes one picture of the view there
once the scene settles (the renderer's scene readback, `request_capture`);
`PRINTCAD_AUTOSAVE_SECS` shortens the autosave wait (`app/recovery.rs`:
copies of edited tabs in the data folder's `recovery/`, taken away when
saved, closed or on exit, offered on the start page after a crash);
`PRINTCAD_BENCH_WORKBENCH=<bench id>` switches to that bench once the
document has a body (so `PRINTCAD_BENCH_TOOL` reaches its tools);
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
`on_command`) · wheel = zoom · LMB = select (click sketch → tree-select; click
solid → face-first, double click → the whole body the face belongs to, one
part of an assembly, and in Design the tree opens to its row; LMB drag
in sketch = box select; ctrl = additive: Ctrl+click adds or removes a
face of the selected body (`session.earlier_faces` beside the last,
`last_face_hit`; `input::toggled`), and an edge, faces and edges mixing;
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
- The suite has a time budget (`scripts/test-budget.mjs`: 60 s a program,
  150 s in all, on CI); a test that needs a big model is `#[ignore]` with
  the reason, run on request. While working, test what a change reaches:
  `scripts/affected-crates.mjs` (changed crates and everything depending
  on them, dev-dependencies included); CI runs everything.

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
  feature's name (`execute_solid_chain_named`, the worker's tags). The
  mesh carries `face_names` and `edge_faces`; a pick keeps them
  (`FaceRef::name`, `EdgeRef::faces`, `FacePick`, `EdgePick`, datum
  anchors, `FaceSupport` of a sketch placed on a face), and a rebuild finds
  a named face or edge by name first (`naming::find_face`/`find_edge`,
  reading the running solid's names the chain sets for the op), falling
  back to the stored point + normal. A sketch placed on its body's face
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
  taper (`thread_cut` in `wb_design/src/build.rs`). Drill points and
  counterdrill cones are kernel cone primitives cut at each centre.
  `docs/HOLES.md` is the user guide.
