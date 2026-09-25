# Writing a workbench

A workbench is a crate that implements `core_document::Workbench`. The
application never refers to a workbench by name. Everything a workbench
shows, draws, picks, rebuilds or asks for goes through this trait and the
workbench registry (`DocumentService`).

Part Design (`crates/workbenches/wb_part`) and the Sketcher
(`crates/workbenches/wb_sketch`) are complete examples.

## 1. Create the crate

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

The `egui` feature enables the panel methods. Panel code takes colours and
sizes from `ui_kit::tokens`, never as literal values.

Register the workbench in `crates/workbenches/src/lib.rs`:

```rust
core_document::define_workbenches!(SketchWorkbench, PartDesignWorkbench, MyWorkbench);
```

The order matters. A new document opens in the first workbench that is not
modal, and Preferences lists workbenches in this order.

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
  have one owner only; registration fails otherwise. Part Design also owns
  `core.datum`.
- `.modal()` marks an editing session, like the Sketcher. Entering it
  remembers the previous workbench, and leaving returns there.

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
- The application uses 0 to 6, O, P, F, Shift+F, H, Space and Delete
  without modifiers. A workbench key on one of them hides it while the
  workbench is active.
- A workbench that takes typed numbers from the viewport returns `true`
  from `takes_numeric_input` meanwhile, so the digit keys, `.`, `,` and
  `-` reach it rather than their shortcuts.
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
        "mine.slab" => { /* read a.number("width")?, edit ctx.document */ Ok(json!(id)) }
        _ => Err(CommandError::Unknown(id.to_string())),
    }
}
```

- The host checks the arguments against the spec before the call.
- A command changes the document through its mutators, so the change is
  undoable like a click. It never opens a task or waits for input.
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
/// `node_revision(node)` gives a revision that changes with the data.
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
```

Double clicking a feature in the tree switches to its owner and makes the
feature the active document object. `locks_view_to_plane` keeps the camera
square to the plane while editing.

## 7. Build solids

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

A `BuildPlan` is a list of `kernel_api::SolidOp`s with the feature that
made each one. The application runs it on the kernel thread. A failure is
shown on the feature named in `BuildError::feature`.

## 8. Add menu entries

```rust
fn menu_items(&self, scope: &MenuScope, doc: &Document) -> Vec<MenuItem>;
fn on_command(&mut self, id: &str, scope: &MenuScope, ctx: &mut WorkbenchRuntimeContext) -> bool;
```

The scopes are the right-click menu on a body, a feature row in the tree,
a body row in the tree, the Edit menu, and the start page. A start page
item becomes a New card. Its command runs in a fresh document with one
body.

## 9. Ask the host for things

Every method gets a `WorkbenchRuntimeContext`: the document, the camera
and viewport, hover and selection, the active document object, projection
helpers, and logging (`log_info` and others).

Anything else goes through a request:

```rust
ctx.request(HostRequest::ActivateTool("mine.select".into()));
ctx.request(HostRequest::SelectBody(body));
ctx.request(HostRequest::JournalLabel("Create thing".into()));
ctx.request(HostRequest::StartOn { workbench: WorkbenchId::from("wb.sketch"), attach });
ctx.request(HostRequest::SwitchWorkbench(WorkbenchId::from("wb.part")));
ctx.request(HostRequest::OrientCamera(CameraOrientRequest { .. }));
ctx.request(HostRequest::FinishEditing);
```

The host applies requests after the method returns. Requests from
`on_activate` and `on_deactivate` that switch workbench are ignored, since
those run during a switch. `StartOn` switches workbench and passes `attach`
to the new one as `ctx.attach_request`.

## 10. Draw panels

- `task()` opens the task panel on the right; `ui_task_panel` draws it and
  handles OK and Cancel. One task is one undo step.
- `ui_left_panel` draws under the model tree.
- `viewport_hud` and `status_items` fill the viewport corners and the
  status bar.
- `get_overlay_meshes` and `get_screen_space_overlays`, `_marks` and
  `_labels` draw over the scene while the workbench is active.
- `ui_settings` draws the workbench's page in Preferences.

## Checklist

1. `descriptor` with `icon` and `feature_kinds`, and a test that the icons
   exist.
2. `configure` with the tools and their default keys, and
   `is_tool_enabled` where tools have preconditions.
3. `feature_info`, plus `passive_geometry`, `pick_feature`,
   `delete_feature` and `property_hints` as needed.
4. `rebuild_jobs`, `invalidate_body` and `invalidate_all` if it builds
   solids.
5. `on_input` for the tools.
6. `task` and `ui_task_panel` for editing, `ui_settings` for preferences.
7. `menu_items` and `on_command` for menus and start cards.
8. `register_command` and `run_command` for what scripts can do, and
   `register_import` for files it reads.
9. Registration in `crates/workbenches/src/lib.rs`.

`crates/app_shell/src/app/seam_lint.rs` fails if a workbench name appears in
the application, and CI checks the same.
