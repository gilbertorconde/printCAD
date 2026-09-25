# RFC 0001: WebAssembly workbenches

- Status: draft
- Date: 2026-09-25
- Scope: `core_document` (workbench seam), a new `wb_wasm` crate, `ui_kit`,
  the `workbenches` facade, the plugin SDK

## Summary

Third parties can ship workbenches for printCAD as WebAssembly components.
A workbench package holds one `.wasm` component, a manifest and its icons;
it runs on every platform, inside the app, sandboxed. The host loads each
package into a `WasmWorkbench` that implements the existing `Workbench`
trait, so the rest of the application cannot tell a plugin from a built-in
bench.

The interface is a WIT world, `printcad:workbench`, that mirrors the
`Workbench` trait with three changes: user interface is declared as data
instead of drawn with egui, the document is read through queries and
changed only through commands, and long work runs as jobs off the UI
thread.

## Motivation

The `Workbench` trait already is the only seam between the host and a
bench (`app/seam_lint.rs` enforces it), and it covers everything a bench
does: tools, feature kinds, parametric rebuilds, task panels, overlays,
menus, commands and preferences. But a bench today is Rust code compiled
into the application. Nobody outside the project can add one.

The workbenches people want to add range from small (a gear generator, a
fastener library) to large (CAM: tool libraries, operation lists, toolpath
computation, simulation, G-code post-processors). The design has to carry
both. CAM sets the bar: heavy computation, its own panels, file output.

## Goals

- A third party can write a complete workbench (tools, parametric feature
  kinds, task panels, viewport feedback, commands, preferences) without
  changing or rebuilding printCAD.
- One package runs on every platform the app runs on.
- A plugin cannot crash the application, hang the interface, or reach files,
  network or processes it was not granted.
- Plugin features are ordinary document features: undo, recording,
  scripting, formulas, the AI tools and document sync work with them
  unchanged.
- A document that uses a missing plugin still opens and shows its last
  built shapes.
- The built-in benches can move onto the same interface over time, so there
  is one seam, not two.

## Non-goals

- Arbitrary egui drawing from plugins. Plugins declare their interface; the
  host draws it.
- Direct access to kernel shapes. Plugins describe operations; the kernel
  runs them.
- Plugin-to-plugin calls, other than through the command registry.
- A marketplace. This RFC covers the package format and installing from a
  file; distribution comes later.

## Background: the seam today

What a bench exchanges with the host falls into four groups. The design
below is shaped by which of them cross a WebAssembly boundary easily.

1. **Plain data, already serializable.** Descriptors, `ToolDescriptor`,
   `MenuItem`, `TaskInfo`, `FeatureInfo`, `Parameter`, `CommandSpec`,
   command arguments and results (JSON), `SolidOp` and `BuildPlan`
   (`kernel_api`, serde throughout), overlays, marks, labels, the HUD and
   status items. These cross as they are, with `&'static str` fields turned
   into owned strings.
2. **Direct document access.** Hooks receive `&mut Document` or a
   `WorkbenchRuntimeContext` holding one. A plugin cannot hold a reference
   into host memory. It reads through queries and writes through commands,
   which is also the path Lua scripts and AI agents use, so every change is
   a recorded op with undo.
3. **egui.** `ui_task_panel`, `ui_settings` and `ui_left_panel` draw with
   `egui::Ui`. This does not cross. Plugins declare panels as data.
4. **Opaque state.** `suspend_session` and `resume_session` pass
   `Box<dyn Any>`. Plugins pass bytes instead.

The heavy geometry never needs to cross. `rebuild_jobs` already returns a
`BuildPlan` of `SolidOp`s that the kernel worker runs natively, and the only
fine-grained kernel calls are `KernelQueries` (edge projection, overlap).
This is why a WebAssembly bench does not pay per kernel operation.

## Design

### Components

- `wb_wasm` (new crate, below `app_shell`): the wasmtime engine, package
  loading, the `WasmWorkbench` adapter, the host side of the WIT world,
  capability enforcement, job threads.
- `workbenches` facade: registers the built-in benches and then every
  installed package through `wb_wasm`, so `app_shell` still names no bench
  and no plugin crate.
- `ui_kit`: a panel renderer that draws the declared panel schema with the
  existing widgets (`QtyField`, `FormulaField`, check rows, select fields,
  tables), and a runtime icon registry beside the vendored table.
- `printcad-bench-sdk` (new crate, published): Rust bindings generated from
  the WIT world, a `Bench` trait close to `Workbench`, helpers for building
  `SolidOp` lists, panels and overlays, and an example bench.

### Package format

A package is a zip file with the extension `.pcbench`:

```
bench.toml          the manifest
bench.wasm          the component
icons/*.svg         icons, referenced by name
README.md           shown on the plugin's Preferences page
```

The manifest:

```toml
id = "acme.cam"                  # reverse-domain, unique
name = "CAM"
version = "0.3.0"                # the plugin's own version
api = "printcad:workbench@0.1"   # the WIT package it targets
feature_kinds = ["acme.cam.op"]
description = "Toolpaths and G-code for milling"

[capabilities]
files = ["data"]                 # its own data folder only
save_dialog = true               # may ask the user where to save
helper = false                   # may run a native helper (see Jobs)
network = false
```

Packages install into `settings::workbenches_dir()/<id>/`, unpacked, and are
loaded at startup. Preferences lists them, shows the capabilities each was
granted and can disable or remove one.

### The WIT world

The world imports host interfaces and exports the bench. Types follow the
current Rust ones; JSON travels as strings where the schema belongs to
the bench (feature data, command arguments, settings).

```wit
package printcad:workbench@0.1.0;

interface types {
  type feature-id = string;          // uuid
  type body-id = string;
  record vec3 { x: f64, y: f64, z: f64 }
  record face-ref { point: vec3, normal: vec3, surface: option<string> }
  record edge-ref { point: vec3, direction: vec3, length: f64, body: body-id }
  record tool { id: string, label: string, icon: string, row: u8,
                category: option<string>, shortcut: option<string>, toggle: bool }
  record descriptor { id: string, label: string, description: string,
                      icon: string, feature-kinds: list<string>, modal: bool }
  record feature-info { icon: string, kind-label: string, builds-solid: bool }
  record parameter { key: string, name: option<string>, label: string,
                     dim: string, pointer: string, scale: f64, integer: bool }
  record command-spec { id: string, summary: string, params-json: string,
                        returns: string, read-only: bool }
  variant build-result { plan(string), error(string) } // plan: SolidOp list as JSON
  record rebuild-job { body: body-id, result: build-result }
  // overlays, marks, labels, hud, status, menu items, panels: see below
}

interface host {
  use types.{feature-id, body-id};
  // Reading the document.
  feature-data: func(id: feature-id) -> option<string>;      // evaluated values
  features-of-kind: func(kind: string) -> list<feature-id>;
  feature-body: func(id: feature-id) -> option<body-id>;
  body-placement: func(body: body-id) -> list<f64>;          // row-major 4x4
  body-mesh: func(body: body-id) -> option<mesh-handle>;      // see Meshes
  selection: func() -> string;                                // JSON
  // Changing it: every change is a command, recorded and undoable.
  call: func(command: string, args-json: string) -> result<string, string>;
  // Kernel questions.
  project-edge: func(body: body-id, near: vec3, plane-json: string) -> result<string, string>;
  // Talking back.
  request: func(request-json: string);   // HostRequest: tool, selection, camera, journal label
  log: func(level: string, message: string);
  redraw: func();
}

interface bench {
  use types.{descriptor, tool, feature-info, parameter, command-spec, rebuild-job};
  descriptor: func() -> descriptor;
  tools: func() -> list<tool>;
  commands: func() -> list<command-spec>;
  feature-info: func(kind: string, data: string) -> feature-info;
  parameters: func(kind: string, data: string) -> list<parameter>;
  settle: func(kind: string, values: string) -> string;
  rebuild: func(bodies: list<body-id>) -> list<rebuild-job>;
  run-command: func(id: string, args-json: string) -> result<string, string>;
  on-activate: func();
  on-deactivate: func();
  on-input: func(event-json: string, tool: option<string>) -> bool;
  frame: func() -> frame-state;                  // everything drawn this frame
  task: func() -> option<string>;                // TaskInfo as JSON
  panel: func(which: panel-slot) -> list<widget>;
  panel-event: func(which: panel-slot, event: widget-event);
  menu-items: func(scope-json: string) -> list<string>;
  on-menu: func(id: string, scope-json: string) -> bool;
  delete-feature: func(id: string) -> bool;
  settings: func() -> option<string>;
  apply-settings: func(json: string);
  suspend: func() -> option<list<u8>>;
  resume: func(state: option<list<u8>>);
}

world workbench {
  import host;
  export bench;
}
```

`widget`, `widget-event`, `panel-slot` and `frame-state` are defined in the
next sections. The exact WIT is settled in milestone 1 against the example
benches; the shape above is the commitment.

### Mapping from the `Workbench` trait

| Trait | WIT | Change |
|---|---|---|
| `descriptor`, `configure` | `descriptor`, `tools`, `commands` | icons by name from the package |
| `feature_info` | `feature-info` | the host passes the node's data |
| `parameters`, `settle`, `values_moved` | `parameters`, `settle`, a `values-moved` event | none |
| `rebuild_jobs`, `invalidate_body`, `invalidate_all` | `rebuild` | the host decides which bodies are dirty, passes them, and settles the dirty flags itself; the bench only plans |
| `run_command` | `run-command` | edits inside it go through `host.call` |
| `on_input`, `on_frame` | `on-input`, `frame` | events as data, see Input |
| `task`, `ui_task_panel`, `ui_settings`, `ui_left_panel` | `task`, `panel`, `panel-event` | declared panels |
| `viewport_hud`, `status_items`, overlays, marks, labels, overlay meshes | `frame` | one call, cached |
| `menu_items`, `on_command` | `menu-items`, `on-menu` | none |
| `delete_feature` | `delete-feature` | through commands |
| `settings_json`, `apply_settings_json` | `settings`, `apply-settings` | none |
| `suspend_session`, `resume_session` | `suspend`, `resume` | bytes instead of `Box<dyn Any>` |
| `passive_geometry`, `pick_feature` | milestone 3 | a bench that draws its own features (like sketches) |

### Document access

A plugin reads the document through `host` queries. `feature-data` answers
the evaluated values (`Document::feature_values`), the data the feature
builds from, so formulas work for plugin features as for built-in ones.

A plugin changes the document only through `host.call`, which runs a
registered command with the same checks and recording as a Lua script or an
AI agent. Two host commands give a bench what it needs for its own
features:

- `doc.add_feature {kind, body, name, data}`: adds a feature of one of the
  bench's kinds (refused for a kind the bench does not own).
- `doc.set_feature_data {id, data}`: replaces an owned feature's data.

Everything a plugin does is therefore an op in the journal: one undo step
per gesture, replayed to peers through the document server, recorded when
recording is on.

Topological references stay the host's business. A plugin that stores a
reference to a face or an edge stores the `face-ref` or `edge-ref` the host
gave it and hands it back in its plan; the kernel resolves it at build time
(`EdgeSelection::Near`, face picks), exactly as Part Design does. When the
host's references improve, plugins improve with them.

### Rebuilds

Each frame, for every bench whose feature kinds have dirty features, the
host collects the dirty bodies and calls `rebuild` with them. The bench
answers a plan per body, `SolidOp`s as JSON. The host clears the dirty flags
of the features it passed, then hands the plans to the kernel worker as it
does for Part Design. A plan error attaches to the feature named in the
result, as `BuildError` does today.

A bench whose feature must start from another body's solid uses
`SolidOp::Shape` with the blob it gets from `host.call("doc.body_shape", …)`,
a read-only command, rather than holding shapes.

### Declared user interface

Panels are lists of widgets. The host draws them with `ui_kit`, so plugin
panels look like built-in ones, follow the theme and support formulas.

```wit
variant widget {
  heading(string),
  text(string),
  note(tuple<string, string>),               // kind (info, warning, error), text
  number(number-field),                      // a parameter or a local value
  choice(choice-field),
  toggle(toggle-field),
  button(button-field),                       // sends a panel event
  pick(pick-field),                          // "click a face", "click an edge"
  table(table-field),                        // rows and columns, cells editable
  list(list-field),                          // selectable rows, e.g. operations
  group(group-field),                        // a titled, collapsible section
  progress(progress-field),                  // a running job
}
```

A `number` field bound to a parameter key is a formula field: the host
reads the parameter, shows its formula, and writes the value back through
`doc.set_feature_data`, or the formula through `doc.set_formula`. A plugin
never handles formulas itself.

Panel slots are the task panel, the bench's Preferences page and the left
panel. Events come back as `widget-event` (a field changed, a button
pressed, a row selected, a pick made) with the widget's id. The host asks
for the panel again after an event and at most once per frame otherwise.

Viewport feedback is the existing overlay data, plus one addition:

- `polyline3d { points, color, width, dashed }`: a world-space line strip.
  Toolpaths need it; datum guides and the sketcher's passive drawing would
  use it too.

### Input and picks

`on-input` receives the event as JSON: presses, releases and moves with the
viewport position, the world position under the cursor and the body there,
keys, tool activation and registered actions. Picks come with the event
(`face-ref`, `edge-ref`, both in world space and in the body's frame), as
`ctx.selected_face` and `ctx.selected_edges` give them to built-in benches.
The answer says whether the event was consumed.

### Frame state and budgets

The host must never wait on a plugin to draw a frame. `frame` returns
everything the bench shows this frame (HUD, status items, overlays, marks,
labels, meshes), and the host caches it. It calls `frame` again only after
an event reached the bench, after a document change, or when the bench
called `host.redraw` (an animation). A frame with none of these reuses the
cache.

Every call runs under a wasmtime epoch deadline: 20 ms for frame and input
calls, 1 s for commands, panel events and `rebuild`. A call that overruns is
stopped, the bench is marked as misbehaving, logged, and its last cached
state is shown. Three overruns in a session disable the bench until the
user enables it again. Memory per instance is capped (1 GB by default,
per-package in the manifest up to the 4 GB WebAssembly limit).

### Jobs

Work longer than a call budget runs as a job. A bench starts one with
`host.call("job.start", {entry, input})`; the host runs the bench's exported
`job-run(input) -> result` on a worker thread, in a separate instance of the
same component, with the kernel-worker style progress and cancel wiring
(the `Activity` slot and the status bar's Cancel). The result arrives as a
`job-finished` input event. A CAM bench computes toolpaths this way without
blocking the interface.

Some computations need every core, which WebAssembly components do not yet
provide well. A package granted the `helper` capability ships native helper
programs per platform (`helpers/<os>-<arch>/<name>`), and a job may run one
through `host.call("job.helper", {name, input})`. Input and output travel as
bytes over the helper's stdin and stdout. Helpers are the one place a
plugin runs native code, which is why the capability is off by default and
shown to the user at install.

### Meshes

Toolpath work needs the model's triangles. `body-mesh` returns a resource
handle; the bench reads it in chunks (`positions(start, count)`,
`indices(start, count)`) into its own memory, or passes the handle to a job,
which receives the mesh copied into the job's instance once. Shapes
themselves never cross.

### Capabilities and sandbox

- WASI file access is limited to the package's data folder
  (`workbenches_dir()/<id>/data`). Other files go through the host's save
  dialog (`HostRequest::SaveFile`) or an open dialog request, so the user
  chooses every file a plugin reads or writes outside its folder.
- No network unless granted.
- No processes, except helpers under the `helper` grant.
- Clipboard through the host only.

The manifest lists what a package wants; the user approves it at install,
and Preferences shows and revokes grants.

### Missing and disabled plugins

A feature whose kind no loaded bench claims:

- keeps its data untouched in the document and in saves;
- is drawn from the last built solid the file carries (`brep/<uuid>.bin`),
  with no rebuild attempted;
- shows in the tree with the kind's name and a note naming the missing
  workbench (`acme.cam 0.3.0`), and cannot be edited or deleted;
- makes the bodies it belongs to read-only for other benches' features,
  since rebuilding them would drop its contribution.

Each feature records the plugin id and version that made it
(`FeatureNode::made_by`, new, `#[serde(default)]`).

### Versioning

- The WIT package is versioned. The host supports the current major version
  and the one before it through adapter shims, and refuses others with a
  clear message.
- A plugin's own data changes are its business: the host calls
  `migrate(kind, data, from-version) -> string` (an optional export) when
  it loads a feature made by an older version of the plugin.
- Command ids of a plugin are namespaced by its id (`acme.cam.pocket`) and
  checked for clashes at load, as the registry checks feature kinds today.

### Settings and sessions

A bench's settings are JSON the host stores in the user's settings, as
`settings_json` works today. Per-tab editing state goes through `suspend`
and `resume` as bytes, so switching tabs works for plugins as for the
sketcher.

## Changes to printCAD before plugins (dogfooding)

The plugin interface is only proven when a built-in bench uses it. The
Assembly bench, the smallest and newest, moves first:

1. **Declared panels.** Its task panels and Preferences page become panel
   lists drawn by the `ui_kit` renderer. The renderer covers the widget set
   above; Part Design's editors follow later.
2. **Commands for every edit.** Its hooks stop mutating `Document` directly
   and call commands through the same `call` path a plugin uses.
3. **Owned strings.** `&'static str` fields in `ToolDescriptor`,
   `FeatureInfo`, `MenuItem`, `ViewportHud` and the overlay types become
   `Cow<'static, str>`, so plugin data fits without leaking.
4. **Runtime icons.** `ui_kit::icon` gains a registry for icons loaded at
   runtime from packages, beside the vendored table.
5. **World-space polylines** in the overlay data.
6. **Jobs** in the kernel worker, usable by built-in benches too
   (interference checks would move onto them).

Each lands on its own, with the bench's behaviour unchanged.

## Performance budget

- A WebAssembly call into a bench: under 1 µs plus the data it carries.
- A frame: at most one `frame` call per bench that asked for one; cached
  otherwise. Serializing a typical frame state (a few hundred overlay
  segments) costs tens of microseconds.
- A rebuild: one `rebuild` call per dirty body set; the kernel work is
  native and unchanged.
- Compute inside a bench: within 1.5 to 2 times native for typical code;
  helpers for anything that must use every core.

Milestone 2 measures these on the example benches before the interface is
frozen.

## Alternatives considered

- **Lua workbenches.** The least code (mlua is already here) and the same
  command surface. Rejected as the main mechanism: interpreted speed rules
  out real toolpath computation, and a restricted Lua is not a sandbox.
  Scripts stay the way to automate, not to extend.
- **Native libraries** (`cdylib` with a C interface, or `abi_stable`).
  Native speed and every core, but one binary per platform, no sandbox, and
  a crash in a plugin takes the application down.
- **Separate processes** (JSON-RPC, with rkyv or shared memory for bulk
  data). Crash isolation, native speed, any language, and the framing code
  already exists for AI agents. Rejected as the main mechanism for calls
  per event and per frame (5 to 20 µs each, a process per plugin, one
  binary per platform, no sandbox), but kept, as helpers, for the jobs that
  need it.

## Risks and open questions

- **Threads.** CAM wants parallel toolpath computation; WebAssembly threads
  in components are immature. Helpers cover it now; revisit when the
  component model's threading settles.
- **Panel expressiveness.** The widget set must be rich enough for CAM
  (tool tables, operation lists, simulation controls). The CAM prototype in
  milestone 3 is the test; widgets are added where it falls short.
- **wasmtime's cost.** A large dependency and longer builds. It lives in
  `wb_wasm` only; the rest of the workspace does not build against it.
- **Debugging plugins.** Authors need logs, panics with messages and a way
  to reload a package without restarting. Milestone 2 includes a reload
  command and panic capture into the log.
- **Topological naming.** Plugins inherit the host's geometric references
  and their limits until the host has better ones.
- **Undo inside long jobs.** A job's result lands as commands when it
  finishes; a job whose inputs changed meanwhile must be discarded or
  rerun. The job API carries the document's edit count it started from.

## Milestones

0. **Dogfooding** (in tree, no WebAssembly): declared panels, commands for
   every edit, owned strings, runtime icons, world-space polylines, jobs,
   with Assembly moved onto them.
1. **Host adapter.** `wb_wasm`, the WIT world, loading packages from a
   folder, `WasmWorkbench`, and the SDK with a "Gear" example bench: one
   parametric feature, one tool, one task panel.
2. **Complete surface.** Input and picks, frame caching and budgets, menus,
   settings, sessions, reload, the performance measurements.
3. **Heavy work.** Jobs, meshes, helpers, capabilities and the install flow;
   a CAM prototype bench that computes a 2D pocket toolpath, draws it and
   writes G-code.
4. **Stable 1.0.** Missing-plugin handling, migrations, the versioning
   policy, the SDK documentation and a plugin guide beside
   `WORKBENCH_GUIDE.md`.

## Acceptance

The RFC is done when the Gear and CAM example benches, built only against
the published SDK, install from a `.pcbench` file on Linux and on the other
supported platforms, and:

- their features are parametric (formulas drive them), undoable, recorded,
  scriptable and synced like built-in features;
- a CAM job runs without stalling the interface and can be cancelled;
- killing the plugin mid-call (a trap, an overrun) leaves the app running;
- a document using them opens without them, showing their last shapes.
