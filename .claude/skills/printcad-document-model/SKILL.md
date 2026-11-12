---
name: printcad-document-model
description: "The document model and its storage: core_document (Document, feature tree, ops, datums, the registry), feature_walk, renamed ids, doc_server and printcad-serverd, local_ipc, doc_render (thumbnails, PDF reports, print layout packing), and the body data built on them: linked copies and arrays, notes, body placement, variables and formulas, and how a task maps onto undo. Use when adding or changing a Document mutator, op or persisted field, a body property, formulas or configurations, a task's undo behaviour, or code in crates/core_document, crates/doc_server, crates/local_ipc, crates/doc_render or crates/feature_walk. Not for the Workbench trait seam (use printcad-workbench-api) or solid builds (use printcad-kernel)."
---

# printCAD: document model

The invariants in `CLAUDE.md` still bind here; this skill never weakens one. The op, undo, document-server and `.prtcad` invariants are the ones most often in play.

## Crates

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
- `feature_walk`: `#[derive(Walk)]` for `core_document::walk::Walk`: each
  field of a feature type walked by the role its `#[walk(...)]` names (a
  number formulas set, a list of them, a setting, a function) or by its
  own type, at the JSON pointer serde writes it to (it reads `untagged`,
  `flatten` and `rename`); a `FeatureId` is a feature read. Design's
  `DesignFeature` derives it: its `parameters` and `dependencies` are
  the walk's (`Walker::of`), and a field with no role fails to compile.
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

## Body data

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

## Variables, tasks and undo

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
