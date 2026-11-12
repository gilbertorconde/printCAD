---
name: printcad-workbench-api
description: "The seam between the host and every bench: the Workbench trait (descriptor, tools, tasks, floating cards, viewport(), ViewportOutput, feature_info, references, rebuild_jobs, menus, imports), bench_api (the package contract), wb_wasm and workbench packages (sdk/, install, stores, updates, made_by), the workbenches facade and bench_fixtures, and check-workbench-seam.mjs. Use when adding or changing a Workbench method or what it returns, a bench_api type, a HostRequest a bench needs, package loading or the store, or code in crates/bench_api, crates/workbenches/wb_wasm, crates/workbenches/src, crates/workbenches/fixtures or sdk/. Not for one bench's own features (use printcad-benches or printcad-sketcher)."
---

# printCAD: the Workbench seam and packages

The invariants in `CLAUDE.md` still bind here; this skill never weakens one. The UiLayer / `HostRequest` invariant governs how a bench talks back to the host.

## The Workbench trait

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
the drawing side's `ui_task_panel()` (`bench_panel::BenchUi`, as are
`ui_settings`, `ui_left_panel` and `ui_viewport_strip`) own the right panel (`TaskRequest` in, `TaskOutcome` out);
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
undo in `printcad-document-model`); the viewport's `faded`
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

## Crates

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
- `workbenches` facade: `register_all_workbenches` (built-in benches, the
  listed ones then `ProjectImportBench`) and
  `register_packages` (installed packages, after them, as the user allowed;
  `PackageStatus` for the Preferences page).
- `workbenches/fixtures` (`bench_fixtures`): ready-made scenes built from
  the benches' feature types (a dimensioned sketch, padded, pocketed) for
  the app's `PRINTCAD_BENCH_SKETCH` hook and tests; the host composes
  benches only through it, never by naming them. `scripts/check-workbench-seam.mjs`
  rejects concrete bench references in `app_shell/src` and direct bench
  dependencies; local gates and CI run the same checker.

## Building and managing packages

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
