# Writing a workbench

A workbench is a crate that implements `core_document::Workbench`. The
application host knows no workbench by name: everything a bench shows,
draws, picks, rebuilds, deletes, offers in a menu or asks of the host
goes through that trait and the registry (`DocumentService`). This guide
walks through the surface in the order a new bench needs it. Part Design
(`crates/workbenches/wb_part`) and the Sketcher
(`crates/workbenches/wb_sketch`) are the reference implementations.

## The crate

```toml
[package]
name = "wb_mine"

[features]
default = ["egui"]
egui = ["core_document/egui", "dep:egui", "dep:ui_kit"]

[dependencies]
core_document = { path = "../../core_document" }
kernel_api = { path = "../../kernel_api" }
serde = { workspace = true }
serde_json = { workspace = true }
egui = { workspace = true, optional = true }
ui_kit = { path = "../../ui_kit", optional = true }
```

The `egui` feature gates the panel hooks; a bench with no panels needs no
`egui` dependency at all. Colours and sizes in panel code come from
`ui_kit::tokens`, never as literals.

Register the crate in `crates/workbenches/src/lib.rs`:

```rust
core_document::define_workbenches!(SketchWorkbench, PartDesignWorkbench, MyWorkbench);
```

Registration order matters twice: the first bench that is not an edit
session is where a new document lands, and the Preferences rail lists the
benches in this order.

## Saying what the bench is

```rust
fn descriptor(&self) -> WorkbenchDescriptor {
    WorkbenchDescriptor::new("wb.mine", "Mine", "What it is for.")
        .icon("workbench-mine")
        .feature_kinds(["wb.mine"])
}
```

- `icon` names a drawing in the design set (`ui_kit::icon`), used by the
  bench switcher, the menu and the Preferences rail. Add icons through
  `scripts/vendor-icons.mjs`; a test in each bench asserts every icon it
  names exists.
- `feature_kinds` are the `FeatureNode::workbench_id` values the bench
  **owns**: it presents, renders, picks, edits and deletes features of
  those kinds. A bench that stores features lists its own id. A kind can be
  claimed once; registration fails otherwise. Part Design also claims
  `core.datum`, the document's own datum features.
- `.modal()` marks an edit-session bench (the Sketcher): entering it from
  another bench remembers that bench to return to when the session ends,
  and it is never the landing bench.

## Tools, the toolbar and the bench menu

`configure` runs once at registration and declares tools:

```rust
fn configure(&self, context: &mut WorkbenchContext) {
    context.register_tool(
        ToolDescriptor::new_action("mine.thing", "Thing", Some("shape")).icon("thing").row(1),
    );
}
```

The toolbar draws them (row 0 shares the standard row, rows 1 and 2 are
the bench's own) and the bench's top menu lists them grouped by category.
`new_radio_group`, `new_check` and `new_action` pick the button behaviour;
`.variants(..)` adds a dropdown; `.planned(note)` shows a disabled button
for something designed but not built. `is_tool_enabled` and `tool_toggled`
are asked every frame. An Action tool reaches `on_input` as
`WorkbenchInputEvent::ToolActivated` with the tool id the moment it is
clicked; return `InputResult::consumed()` to clear it.

## Owning features

Store features as a type implementing `WorkbenchFeature` (`workbench_id`,
`to_json`, `from_json`, `dependencies`, `name`) and add them with
`ctx.document.add_feature_in_body(feature, name, body)`. Dependencies
declared there drive dirty propagation.

For every kind the bench claims, the registry asks the bench:

```rust
/// The tree row's icon and labels; the hover card names a body after
/// its last feature with `builds_solid`.
fn feature_info(&self, node: &FeatureNode) -> FeatureInfo;

/// What the scene draws for a feature that is visible and not under
/// edit, with a revision the mesh cache keys on (`node_revision(node)`
/// hashes the payload). The host colours it.
fn passive_geometry(&self, doc: &Document, id: FeatureId, node: &FeatureNode)
    -> Option<PassiveGeometry>;

/// Pixels from the cursor to the feature, when close enough to count.
/// `runtime::viewport_to_plane` and `world_to_viewport` do the projection.
fn pick_feature(&self, doc: &Document, id: FeatureId, node: &FeatureNode, pick: &ViewportPick)
    -> Option<f32>;

/// Remove the feature and settle what depended on it. The default just
/// removes it.
fn delete_feature(&mut self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) -> bool;

/// Which payload fields are lengths (display unit) and which name other
/// features or bodies, for the generic property panel.
fn property_hints(&self) -> PropertyHints;
```

A double click on a feature in the tree activates the bench that claims
its kind and makes the feature the active document object; the bench
picks it up in `on_frame`/`on_input` (the Sketcher's `editing_feature`
comes from there). `locks_view_to_plane` keeps the camera square to the
plane while an edit session is open.

## Rebuilding solids

A bench whose features produce a body's solid implements:

```rust
/// Bodies to rebuild now, each with a plan. Called on every bench each
/// frame. Settle the dirty flags of the plan's features and their inputs
/// here, or the job comes back every frame.
fn rebuild_jobs(&self, doc: &mut Document) -> Vec<RebuildJob>;
/// The body's history changed shape: rebuild from the start, or drop the
/// derived solid when no history is left.
fn invalidate_body(&self, doc: &mut Document, body: BodyId);
/// Every derived solid is stale (history jump, Recompute All).
fn invalidate_all(&self, doc: &mut Document);
```

A `BuildPlan` is a chain of `kernel_api::SolidOp`s with the feature
responsible for each op; the host runs it on the kernel worker and
attributes a failure to `BuildError::feature`. An imported body's solid is
not the history's to drop (`Document::body_solid_is_imported`).

## Menus and start cards

```rust
fn menu_items(&self, scope: &MenuScope, doc: &Document) -> Vec<MenuItem>;
fn on_command(&mut self, id: &str, scope: &MenuScope, ctx: &mut WorkbenchRuntimeContext) -> bool;
```

Scopes: the viewport's right-click menu on a body, a tree feature row, a
tree body row, and the start page's New cards. The host draws its own
entries first, then every bench's, in registration order, and runs
`on_command` on a pick. A start-page item becomes a card; its command
runs in a fresh document with one body, in the bench, so the Sketcher's
"Empty sketch" card is just an item plus a command.

## Talking to the host

Hooks get a `WorkbenchRuntimeContext`: the document (mutable), camera
and viewport facts, hover and selection, the active document object
(read and write: setting it selects the feature), `ctrl_down`, the
selected face, `attach_request` (see below), projection helpers and
logging (`log_info` and friends go to the app's log panel).

Anything else the bench wants of the host is a request:

```rust
ctx.request(HostRequest::ActivateTool("mine.select".into()));
ctx.request(HostRequest::SelectBody(body));
ctx.request(HostRequest::JournalLabel("Create thing".into()));
ctx.request(HostRequest::StartOn { workbench: WorkbenchId::from("wb.sketch"), attach });
ctx.request(HostRequest::SwitchWorkbench(WorkbenchId::from("wb.part")));
ctx.request(HostRequest::OrientCamera(CameraOrientRequest { .. }));
ctx.request(HostRequest::FinishEditing);
```

The host applies a hook's requests in that order once the hook returns,
from every hook site: input, per-frame, panels, activation, the overlay
getters. A lifecycle hook (activate, deactivate) runs inside a switch, so
its switch and start requests are dropped. `StartOn` switches to a bench
and hands it `attach` as `ctx.attach_request` on its next hook; the
receiving bench takes it (`ctx.attach_request.take()`).

## Panels, HUD and status

- `task()` returns `Some(TaskInfo)` to open the right-hand task panel;
  `ui_task_panel` draws its body and answers OK/Cancel/Enter/Esc through
  `TaskRequest` with a `TaskOutcome`. One task is one undo entry.
- `ui_left_panel` draws under the model tree.
- `viewport_hud` (tool hint, badge, legend, footer, OVP card) and
  `status_items` feed the viewport corners and the status bar.
- `get_overlay_meshes` and `get_screen_space_overlays/marks/labels` draw
  for the active bench each frame, world-space and screen-space.
- `ui_settings(ui, filter)` is the bench's Preferences page; the rail
  gets one entry per registered bench automatically.

## Checklist for a bench that owns a feature kind

1. `descriptor` with `icon` and `feature_kinds`; a test that its icons exist.
2. `configure` with the tools; `is_tool_enabled` for the ones with
   preconditions.
3. `feature_info`; `passive_geometry` and `pick_feature` if the feature
   has geometry of its own; `delete_feature` if removing it must settle
   other features; `property_hints` if its payload has lengths or refs.
4. `rebuild_jobs`/`invalidate_body`/`invalidate_all` if it builds solids.
5. `on_input` for the tools, requests for what the host must do.
6. `task`/`ui_task_panel` for feature editing; `ui_settings` for prefs.
7. `menu_items`/`on_command` for contextual entries and start cards.
8. Register it in `crates/workbenches/src/lib.rs`.

The host's own test `crates/app_shell/src/app/seam_lint.rs` fails when a
bench name, id or feature type appears in the host, and CI greps for the
same, so a bench can only ever reach the app through this trait.
