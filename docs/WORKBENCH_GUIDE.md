# Writing a workbench

A workbench is a crate that implements `core_document::Workbench`. The
application never refers to a workbench by name. Everything a workbench
shows, draws, picks, rebuilds or asks for goes through this trait and the
workbench registry (`DocumentService`).

Design (`crates/workbenches/wb_design`), the Sketcher
(`crates/workbenches/wb_sketch`), Surface and Assembly are complete
examples. This guide is for a built-in workbench; one installed as a
package is written against the guest SDK instead ([Workbench
packages](PLUGINS.md)).

## 1. Create the crate

```toml
[package]
name = "wb_mine"

[features]
default = ["egui"]
egui = ["core_document/egui", "dep:egui", "dep:ui_kit"]

[dependencies]
core_document = { path = "../../core_document" }
kernel_api = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
egui = { workspace = true, optional = true }
ui_kit = { path = "../../ui_kit", optional = true }
```

The `egui` feature enables the panel methods. Panel code takes colours and
sizes from `ui_kit::tokens`, never as literal values.

Add it to the workspace members and to the `workbenches` crate's
dependencies, then register it in `crates/workbenches/src/lib.rs`:

```rust
core_document::define_workbenches!(
    SketchWorkbench,
    DesignWorkbench,
    SurfaceWorkbench,
    AssemblyWorkbench,
    MyWorkbench
);
```

The order matters. A new document opens in the first workbench that is not
modal, and Preferences lists workbench pages in this order. The switcher
and menus sort by label.

## 2. Describe the workbench

```rust
fn descriptor(&self) -> WorkbenchDescriptor {
    WorkbenchDescriptor::new("wb.mine", "Mine", "What it is for.")
        .icon("workbench-mine")
        .feature_kinds(["wb.mine"])
}
```

- `icon` names an icon in `ui_kit::icon`. Add icons with
  `scripts/vendor-icons.mjs`.
- `feature_kinds` lists the feature kinds this workbench owns. The owner
  draws, picks, edits and deletes features of those kinds. Each kind can
  have one owner only; registration fails otherwise. Design also owns
  `core.datum`.
- `.modal()` marks an editing session, like the Sketcher. Entering it
  remembers the previous workbench, and leaving returns there.
- `.bought_kinds([..])` names owned kinds whose bodies are bought rather
  than made (catalog hardware): the parts list counts them bought unless
  the user says otherwise, and exports and the slicer leave them out.

## 3. Add tools

`configure` runs once and registers the tools:

```rust
fn configure(&self, context: &mut WorkbenchContext) {
    context.register_tool(
        ToolDescriptor::new_action("mine.thing", "Thing", Some("shape")).icon("thing").row(1),
    );
}
```

- Row 0 shares the standard toolbar row. Rows 1 and 2 belong to the
  workbench. The workbench menu lists tools by category.
- `new_action`, `new_radio_group` and `new_check` choose how the button
  behaves. `.variants(..)` adds a dropdown.
- `.planned(note)` shows a disabled button for a designed tool that is not
  built yet.
- `is_tool_enabled` and `tool_toggled` are called every frame.
- Clicking an action tool calls `on_input` with
  `WorkbenchInputEvent::ToolActivated`. Return `InputResult::consumed()`
  when handled.

## 4. Add keyboard shortcuts

A tool gets a default key with `.shortcut`. Pressing it while the
workbench is active works like clicking the tool's button:

```rust
ToolDescriptor::new("mine.line", "Line", Some("draw")).shortcut("L")
```

For a key that is not a tool, register an action. Its key sends
`WorkbenchInputEvent::Action { id }` to `on_input`:

```rust
context.register_action(
    ActionDescriptor::new("mine.flip", "Flip direction").shortcut("Shift+F"),
);
```

- Keys are written like `L`, `Shift+F`, `Ctrl+Alt+K` or `F5`. A key that
  does not parse panics at registration, so a typo shows up in tests.
- Users can change every key in Preferences › Keyboard. Ids are saved as
  they are, so keep them stable.
- A workbench's keys work only while it is active, and win over the
  application's keys there.
- Keys without Ctrl or Alt are left to text fields while one has focus.
- The application uses 0 to 6, O, P, F, Shift+F, H, F2, F11, Space,
  Shift+Space and Delete without Ctrl or Alt. A workbench key on one of
  them hides it while the workbench is active.
- A workbench that takes typed numbers from the viewport returns `true`
  from `takes_numeric_input` meanwhile, so the digit keys, `.`, `,` and
  `-` reach it rather than their shortcuts. `takes_text_input` does the
  same for every key that types text, letters included, while it types a
  formula.
- To name a key in a hint, implement `shortcuts_changed`. It receives the
  keys in effect by id at start and after every change.

## 5. Add commands

A command is something a script can call by name, with named arguments.
Register it in `configure` and run it in `run_command`:

```rust
context.register_command(
    CommandSpec::new("mine.slab", "Add a slab")
        .param("width", ParamKind::Number, "In millimetres")
        .optional("name", ParamKind::String, "Its name in the tree")
        .returns("the feature's id"),
);

fn run_command(&mut self, id: &str, args: &CommandArgs, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let a = Args(args);
    match id {
        "mine.slab" => {
            let width = a.number("width")?;
            let made: FeatureId = todo!("add a {width} mm slab to ctx.document");
            Ok(json!(made.0.to_string()))
        }
        _ => Err(CommandError::Unknown(id.to_string())),
    }
}
```

- The host checks the arguments against the spec before the call.
- A command changes the document through its mutators, so the change is
  undoable like a click. It never opens a task or waits for input.
- `.read_only()` marks one that changes nothing (an AI agent never waits
  for approval to run it); `.agent_always_asks()` and
  `.agent_never(reason)` restrict what an agent may do with it.
- A tool or panel action that ends in what a command does calls
  `ctx.record(id, args, result)`, so recordings and scripts see the same
  call.
- Ids are unique across the application; registration fails on a
  duplicate.
- Scripts reach it as `pc.mine.slab{width = 20}`.

A command that takes a file as `path` can also serve File › Import.
Register the file kind beside it:

```rust
context.register_import(FileImport::new("Slab drawing", ["slab"], "mine.import"));
```

- The import dialog offers the extensions, and a picked file with one of
  them runs the command with `path` set, recorded as that command.
- A feature id the command answers is selected in the tree.
- Registration fails when the command is not one the workbench registers.

## 6. Store features

Define a type implementing `WorkbenchFeature` (see
[Document model](DOCUMENT_MODEL.md)) and add it with
`ctx.document.add_feature_in_body(feature, name, body)`.

For each kind it owns, the workbench answers these:

```rust
/// Icon and labels for the tree row.
fn feature_info(&self, node: &FeatureNode) -> FeatureInfo;

/// What to draw for the feature when it is visible and not being edited.
/// `node_revision(node)` gives a revision that changes with the data;
/// `region` (a `PassiveRegion`: a profile and an opacity) has the host
/// shade what the lines enclose.
fn passive_geometry(&self, doc: &Document, id: FeatureId, node: &FeatureNode)
    -> Option<PassiveGeometry>;

/// Distance in pixels from the cursor to the feature, if close enough.
fn pick_feature(&self, doc: &Document, id: FeatureId, node: &FeatureNode, pick: &ViewportPick)
    -> Option<f32>;

/// Remove the feature and fix up what depended on it.
/// The default just removes it.
fn delete_feature(&mut self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId) -> bool;

/// Which data fields are lengths and which refer to other features,
/// for the property panel.
fn property_hints(&self) -> PropertyHints;

/// Bodies that are not made (bought parts): an export of every visible
/// body and the slicer leave them out. `bought_kinds` are the kinds every
/// bench declares bought.
fn not_printed(&self, doc: &Document, bought_kinds: &[WorkbenchId]) -> Vec<BodyId>;

/// The parts to print and how many of each, for the print layout. The
/// first bench that answers (the Assembly's parts list) is the one used.
fn print_parts(&self, doc: &Document, bought_kinds: &[WorkbenchId])
    -> Option<Vec<PrintPart>>;

/// Bores the feature threads without cutting the thread, with their
/// designation, for the view, the parts list and STEP.
fn cosmetic_threads(&self, doc: &Document, id: FeatureId, node: &FeatureNode)
    -> Vec<ThreadCallout>;

/// The curves and points the feature lays out, for an array of copies
/// along a path or at points.
fn feature_paths(&self, doc: &Document, id: FeatureId, node: &FeatureNode)
    -> Option<FeaturePaths>;

/// A problem that does not stop the feature building (a sketch whose
/// projected edges are gone), shown on its tree row. Asked whenever the
/// tree draws, so answer from what was last worked out.
fn feature_warning(&self, doc: &Document, kernel: Option<&dyn KernelQueries>, id: FeatureId)
    -> Option<String>;

/// Bodies drawn faded while the bench is active (still pickable): what
/// a tool asks the user to see past.
fn faded_bodies(&self, ctx: &WorkbenchRuntimeContext) -> Vec<BodyId>;

/// Features of other bodies that act on `body` (a joint holding one
/// body to another); the tree lists each under `body` too, as a link.
fn linked_features(&self, doc: &Document, body: BodyId) -> Vec<FeatureId>;

/// Inputs the property panel lets the user swap (a profile sketch), and
/// how to swap one.
fn references(&self, doc: &Document, id: FeatureId, node: &FeatureNode)
    -> Vec<FeatureReference>;
fn set_reference(&mut self, ctx: &mut WorkbenchRuntimeContext, id: FeatureId,
    key: &str, to: ReferenceChoice) -> Result<(), String>;
```

A click on a feature in the tree only selects it: it becomes the active
document object. A double click switches to its owner and calls
`edit_feature`, which is where the workbench opens the feature's task; never
open one on selection alone. `editing_feature` names the feature under
edit (the tree badges it, and a feature that builds solid gets a preview);
`locks_view_to_plane` keeps the camera square to the plane while
`editing_feature` is `Some`. `finish_editing` ends the edit session when
the user finishes it, or when undo takes away the feature under edit.
`shows_origin_planes` draws the origin's planes whatever the view's
setting, for the user to pick one; the plane under the cursor arrives as
`ctx.hovered_base_plane`.

## 7. Take formulas

`parameters` lists an owned feature's numbers that formulas may set and
read: a `Parameter` with a stable key, the name formulas call it, a JSON
pointer into the feature's data, a `Dim` (length, angle, number), a scale
for an angle kept in radians, and whether it is a count. The workbench
builds, draws and edits from `Document::feature_values`, which has every
formula's value in; its own data keeps plain numbers.

- `settle` makes the data whole once values are in (a sketch solves).
- `derive`, `derive_on_solid` and `derive_on_geometry` bring working data
  up to what it follows: another feature, what the last build found of the
  solid (`BuildPlan::probes`), the bodies' geometry. All are derived and
  never recorded.
- `values_moved` records results that follow formulas but are kept rather
  than derived (placements a joint solves for), in the same undo step.

## 8. Build solids

A workbench whose features make a body's solid implements:

```rust
/// Bodies to rebuild now, each with a plan. Called every frame.
/// Clear the dirty flags of the planned features here, or the same job
/// comes back next frame.
fn rebuild_jobs(&self, doc: &mut Document) -> Vec<RebuildJob>;

/// The body's history changed: rebuild it from the start.
fn invalidate_body(&self, doc: &mut Document, body: BodyId);

/// Every solid is out of date (undo, redo, Recompute All).
fn invalidate_all(&self, doc: &mut Document);
```

A `BuildPlan` is a list of `kernel_api::SolidOp`s (`ops`) with the feature
that made each one (`op_features`); the application runs it on the kernel
worker, and a kernel failure is shown on the feature whose op failed. A
feature that cannot be planned stops the plan: put its `BuildError` in
`failed` and the features after it in `unbuilt`, and the body shows the
history before it. `probes` asks the solid questions part way through (where
a face a sketch stands on is now); the answers come back through
`derive_on_solid`. A job whose plan is `Err(BuildError)` puts the error on its
feature and builds nothing; an empty plan drops the body's derived solid.

## 9. Add menu entries

```rust
fn menu_items(&self, scope: &MenuScope, doc: &Document) -> Vec<MenuItem>;
fn on_command(&mut self, id: &str, scope: &MenuScope, ctx: &mut WorkbenchRuntimeContext) -> bool;
```

The scopes are the right-click menu on a body, a feature, body or
component row in the tree, the Edit menu (`edit.cut`, `edit.copy`,
`edit.paste` on the active workbench), and the start page. A start page
item becomes a New card. Its command runs in a fresh document with one
body.

## 10. Ask the host for things

Every hook that acts gets a `WorkbenchRuntimeContext`: the document, the
camera and viewport, hover and selection (`selected_faces`,
`selected_edges`, `selected_face_in(body)` for a body's own frame), the
active document object, the kernel's queries (`kernel`), the colours to
draw in (`sketch_palette`), projection helpers, and logging (`log_info` and
others).

Anything else goes through a request:

```rust
ctx.request(HostRequest::ActivateTool("mine.select".into()));
ctx.request(HostRequest::SelectBody(body));
ctx.request(HostRequest::SelectFeatures(vec![a, b]));
ctx.request(HostRequest::JournalLabel("Create thing".into()));
ctx.request(HostRequest::StartOn { workbench: WorkbenchId::from("wb.sketch"), attach });
ctx.request(HostRequest::SwitchWorkbench(WorkbenchId::from("wb.design")));
ctx.request(HostRequest::OrientCamera(CameraOrientRequest { .. }));
ctx.request(HostRequest::FinishEditing);
ctx.request(HostRequest::SaveFile { name, kind, extension, contents });
ctx.request(HostRequest::RecordAnimation { name, frames, frame_ms });
```

`SelectFeatures` selects several tree rows as Ctrl-clicks would, the
first the active document object. `RecordAnimation` asks where to save
and writes the scene as the camera sees it, once per frame with the
bodies at that frame's placements.

The host applies requests after the method returns, collected with
everything else the hook left (`HookOutcome`). Requests from
`on_activate` and `on_deactivate` that switch workbench are ignored, since
those run during a switch. `StartOn` switches workbench and passes `attach`
to the new one as `ctx.attach_request`.

## 11. Draw panels

- `task()` opens the task panel on the right; `ui_task_panel` draws it and
  handles OK and Cancel. One task is one undo step, unless its `TaskInfo`
  is `stepwise` (the sketcher's session, where each edit is a step).
- `ui_left_panel` draws above the model tree.
- `viewport_hud` and `status_items` fill the viewport corners and the
  status bar.
- `get_overlay_meshes` and `get_screen_space_overlays`, `_polygons`,
  `_images`, `_marks` and `_labels` draw over the scene while the
  workbench is active. Polygons are filled shapes beneath the lines (the
  Assembly's move handles).
- `on_frame` runs every frame while the workbench is active, with the
  time since the last.
- `cancel_pointer_gesture` puts back a held drag when the window loses
  focus, since the release may never arrive.
- `clip_plane` cuts the scene at a plane of any direction while it returns
  one (the sketcher's section view), in drawing and picking alike, standing
  in for the view toolbar's clipping plane.
- `ui_settings` draws the workbench's page in Preferences; `has_settings`
  returning true is what gives it a place there. A bench with nothing to
  set leaves both alone and gets no page. `settings_json` and
  `apply_settings_json` keep its settings in the user's file.
- `suspend_session` and `resume_session` hand the host the editing state
  that belongs to the document on screen, so each tab keeps its own.
- `busy` keeps frames coming while work of the workbench runs away from
  the window.

## Checklist

1. `descriptor` with `icon` and `feature_kinds`, and a test that the icons
   exist.
2. `configure` with the tools and their default keys, and
   `is_tool_enabled` where tools have preconditions.
3. `feature_info`, plus `passive_geometry`, `pick_feature`,
   `delete_feature`, `property_hints`, `references` and `set_reference` as
   needed, and `parameters` for the numbers formulas may set.
4. `rebuild_jobs`, `invalidate_body` and `invalidate_all` if it builds
   solids.
5. `on_input` for the tools.
6. `edit_feature`, `editing_feature`, `task` and `ui_task_panel` for
   editing, `suspend_session`/`resume_session` for per-tab state,
   `has_settings` and `ui_settings` for preferences.
7. `menu_items` and `on_command` for menus and start cards.
8. `register_command` and `run_command` for what scripts can do, and
   `register_import` for files it reads.
9. Registration in `crates/workbenches/src/lib.rs`.

`crates/app_shell/src/app/seam_lint.rs` fails if a workbench crate, id or
feature type appears in the application, and CI checks the same. The
application builds scenes from workbench features only through
`bench_fixtures` (`crates/workbenches/fixtures`).
