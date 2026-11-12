---
name: printcad-app-shell
description: "The app_shell binary's structure and its application-level features: tabs and DocumentSession, the grouped app and tab state, the app/ and ui/ module map (frame, input, commands, recompute, links, kernel_worker, updates, session_store, sixdof), application tasks, unsaved-edit prompts, and the features paragraph (external geometry wiring, File > Export with PMI, Send to slicer, Print layout, save thumbnails, What's new, Edit menu, clipping plane, Measure, print bed, PLANNED placeholders). Use when working in crates/app_shell/src/app or src/ui structure, adding per-tab or app state, export, slicer, print layout, clipping or measure, or a disabled placeholder control. Not for styling and widgets (use printcad-ui-conventions), bindings and picking (use printcad-interaction-model) or frame scheduling (use printcad-render-loop)."
---

# printCAD: the app shell

The invariants in `CLAUDE.md` still bind here; this skill never weakens one. Field order and drop order (`app/gfx.rs`, `PrintCadApp`) and the UI-action dispatch rule are the ones most often in play.

## app_shell

- `app_shell`: binary. **Tabs:** `app/session.rs` is `DocumentSession`,
  everything the app keeps per document (document, journal, file, camera,
  selection, active bench and tool, server connection, in-flight open/save,
  presence, the STEP modal, the benches' suspended editing state); the active
  one sits on `PrintCadApp.session`, the rest are parked in `tabs` and
  `app/tabs.rs` swaps them (`switch_tab`, `open_tab`, `close_tab_interactive`,
  `ensure_fresh_tab`: New and Open reuse a blank tab). Background tabs get
  their turn through `with_tab`/`for_each_tab` (drains, rebuilds, outbox);
  a kernel response routes by body id or by the tab that asked for the
  import (`imports.owner`). **Only a real switch moves bench state**
  (`Workbench::suspend_session`/`resume_session` through the registry); a
  `with_tab` turn must not touch it, save one exception: a script's bench
  command run in a tab off screen (`PrintCadApp::visiting`) lends that one
  bench the tab's kept state for the call and hands it back after
  (`execute_command`, `lend_state`/`return_state`). The app's own state
  outside the tabs is grouped in `app/app_state.rs` (`FrameStats`,
  `GpuInfo`, `InputState`, `Imports`, `FileWork`, `OverlayIds`,
  `SceneMeshes`, `Scripting`, `Autosave`, `Questions`, `Agents`,
  `Packages`, `KeptRun`), so a module borrows one group while the rest
  of the app is borrowed elsewhere; each sits in `PrintCadApp` where its
  members with a thread or a `Drop` stood, since fields drop in
  declaration order. A tab's state is grouped the same way in
  `app/session_state.rs` (`view`, `hover`, `selection`, `file`,
  `connection`, `tools`, `jobs`, `builds`, `drawn` on `DocumentSession`),
  the server connection after the file's save and open receivers and the
  threaded caches after both, as they stood. `app/` modules: `frame.rs` (per-frame loop),
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

## Application features

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
