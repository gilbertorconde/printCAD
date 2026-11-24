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
  (`Derived::store_probe_answers`) that `datum::derive` folds into the
  datum's values, so it and what is built on it follow the solid);
  `derived` (`Derived`, `DerivedMut`): the document's derived build
  state as one field: bodies' meshes (placed and in their own frame),
  shape snapshots and face colours, each feature's `BuildStatus` (error,
  refused edges, warning, unchanged), probe and projection answers, size
  limits, what features were built against, mesh build results, cosmetic
  threads and feature solids. `doc.derived()` reads it; `doc.derived_mut()`
  is the only way to write it (the build paths in `build_driver::store`,
  imports, links, mirrored copies), records no op, and asserts in debug
  builds that nothing records one while it is out. A probe or projection
  answer that moves counts toward `mutation_seq` (Derived's own `moved`
  count), so formulas are worked out again without marking an edit. It
  is saved only as the bodies' meshes, under the JSON's `imported_meshes`
  key (`derived::saved_meshes`), the replicated projection drops that key,
  and a clone keeps all of it;
  `patch` (`DataPatch`): writes into a feature's data, a key set or
  taken away or an item of a list put in, replaced or taken out by its
  id, the payload of `DocumentOp::PatchFeatureData`
  (`Document::patch_feature_data`, beside the whole-payload
  `UpdateFeatureData`); it applies in place, whole or not at all (a
  missing path leaves the data untouched), its inverse worked out as it
  applies (`invert_and_apply`), and consecutive patches of one feature
  join in the outbox and the journal (`DataPatch::then`, `after`);
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
  The derive also implements `core_document::describe::Describe` (each
  field's name, kind from its role, default from serde's attributes; an
  enum's forms with their fields), which `#[derive(Describe)]` gives a
  type of settings alone and `describe::by_serde` a type from a crate
  below; every field type of a described type must be described.
  `describe::render` writes it a line per field: what `describe` and
  `help` list for each Design feature command (`commands/described.rs`).
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
  real spawned daemon (`tests/daemon.rs`). The daemon's `Hub` holds one
  lock (`state`) under which a batch gets its sequence number, its log
  line, its place in the unsaved tail and a message in every client's
  queue (`Ops` to the others, `Acked` to its sender; a writer thread per
  client drains its queue), and a second (`file`, taken first) across a
  file read or write and the tail change that goes with it, so an
  `Opened` reply and the tail after it hold every batch heard before it
  and none after. On the client, `OpBuffer` keeps each unsent op and each
  sent, unconfirmed batch with its inverse, and
  `Document::apply_server_ops` rolls those back, applies the server's
  batch and replays them; an unconfirmed op without an inverse (an
  import) makes it apply over them instead and return false. Each batch
  goes out under a client-made id (`take_pending_batch`, `Ops { batch,
  ops }`); the daemon remembers the ids it ordered (and those its log
  holds, on a restart) and answers one sent again with `Acked` alone. A
  lost connection rejoins (`DocumentSession::rejoin`): `sent_batches` go
  again, then the outbox, then a tab with its own file opens it again
  (`rejoining`); `Acked` and `Ops` heard while it is read wait in
  `held_remote_ops`, and `apply_rejoined_document` puts the read copy in
  place with `carry_unconfirmed_to` (the edits sent after the open on
  top, the journal's open pairs with them), keeping the undo journal,
  view and selection. `Opened` carries the order's `seq` and
  `unsaved_lost` (a restarted daemon that could not read the tail back),
  on which a rejoining tab keeps its copy and lets the tail through `seq`
  go. The
  `Replica` harness in `tests/daemon.rs` drives clients the way the app
  does (`settle` waits for acks, `catch_up` for a sequence number,
  `rejoin` as `DocumentSession::rejoin` does).
  The log is `oplog_path_for(document)` in `oplog_dir()` (the data
  folder's `oplogs/`, `PRINTCAD_OPLOG_DIR` in tests), beside a
  `<key>.oplog.document` note naming the document; homing moves a log
  still beside the document there once (`adopt_legacy_log`).
  Each log line carries its batch's id; every save, homing and last
  leave appends a mark (`{"unsaved": [ids], "file": stamp}`). The tail a
  log holds when a daemon takes up its document (`resume` from
  `<socket>.home` after a daemon died, `take_up_unsaved` on the first
  open; `restore_tail_from`: the last mark's ids plus what follows; a line
  without an id there, or a changed file, means no tail) is not put back
  by itself: it is `State::recoverable`, still named by marks, reported
  as `Opened::recoverable`, brought into the order by `RestoreUnsaved`
  (which a rejoining client sends first), dropped by `DiscardUnsaved` or a
  save. The last leave names nothing unsaved only when that client sent
  `Leave`; the app sends it on close, quit and server switch
  (`DocumentSession::leave_server`), and `DaemonClient`'s drop lets its
  writer finish first. `unsaved_logs` (last line only) feeds the start
  page's Recover list; `discard_unsaved_log` marks a log let go there.
  The log rotates past `--log-bytes` (default 64 MB) into numbered
  generations (`<log>.1`, `<log>.2`, ..., read oldest first), all kept
  until a mark names nothing unsaved, at most `MAX_GENERATIONS`; homing
  the unhomed log takes the log's write lock. The blob store
  (`<log>.blobs`, imports' bytes by hash) keeps what a kept generation
  names: `prune_blobs` runs when such a mark drops generations, on a
  rebase and when the log is homed. The unhomed log
  (`unhomed_oplog_for(socket)`, with a `<log>.socket` note naming an
  untitled socket) goes when its last client leaves with
  `Leave`; after a crash it stays for the next daemon on the socket. A
  daemon ends when its last connection goes once a client has said Hello
  (`Hub::joined`): a look at the socket before then (`listening`, from
  another daemon or `untitled_logs`) does not end it. An untitled tab's
  first save moves it to its file's daemon without a `Leave`
  (`DocumentSession::move_to_server`, `Connection::untitled_log`) and
  sends `AdoptLog { socket }` at once on the move (and on a Save As or a
  rejoin while that save is still to write, and again before each
  `SaveDocument` until one completes; the daemon notes it once,
  `DocumentSession::adopt_untitled_log`), so a file daemon that dies
  while the save packs knows whose its edits are; the log is taken up
  only once the file is written. The file's daemon waits (bounded) for
  the untitled daemon to exit, then carries its log into the document's
  ahead of the save's mark (`set_oplog_home(document, earlier)`,
  `carry_log`). `AdoptLog` also
  writes an `adopt` line naming that socket into the file daemon's
  unhomed log. A file daemon that dies before its first save or open
  leaves its lines there unmarked; the next daemon on that socket sets
  them aside at start (`set_aside_leftover`): they join the untitled log
  an `adopt` line names when it is still kept and unserved, else a mark
  naming none closes them off, so a later open never offers them as the
  file's. Untitled sockets' logs are left alone. A tab closed with
  the save never written drops it (`discard_untitled_log`). After a crash
  `untitled_logs` lists the untitled logs no daemon serves that hold edits
  and do not begin with a `rebased` line (what a `Rebase` writes in an
  unhomed log: its edits apply to a recovered copy, not an empty
  document); `RestoreLog { socket }` on a new untitled tab's daemon
  carries the log into its own and orders its batches like
  `RestoreUnsaved`. The app's recovery list (`file_session::recovery`,
  `Source::UntitledLog`) pairs such a log with its tab's autosave copy
  (`socket_path_for_untitled_of(pid, tab)`) and offers only the log. A client with more than
  `--queue-limit` (default 4096) unwritten
  messages is disconnected; an open's file and tail never count. An
  `AddFeature` or `SetFeatureBody` whose `seq` another feature holds takes
  the next one (`FeatureTree::unclaimed_seq`), so concurrent inserts end
  distinct in the server's order.
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
(`app_ui/src/array_paths.rs`: a bench's `BenchFeatures::feature_paths`, chains and
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
as built and takes the point and label to it, or says it is lost. A note
on a sketch carries `sketch` (`SketchAnchor`: the feature, the element or
constraint id, none for a plane point), `at` and `label` then in the
sketch's plane; the owning bench places it (`BenchFeatures::note_spot`,
the sketcher's `wb_sketch/src/notes.rs`), which `DocumentService::evaluate`
keeps in the derived state (`Derived::sketch_note_spot`, body frame plus
the plane) and `Document::note_placed` reads; a note not yet added is
placed live (`DocumentService::note_placed`). The note tool asks
`note_pick` (the sketcher's hovered item while editing, else the nearest
element, else the plane point), and with it armed a left click in an
edit session is the tool's, not the bench's. The
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
(`BenchParameters::parameters`: a `Parameter` with a stable key, a JSON pointer
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
than derives follow through `BenchParameters::values_moved` (the assembly
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
`HostRequest::KeepTaskEdits` so Cancel stops there. A script's or
agent's run and the person can both edit while the run is open (a task
opened during it, edits between its calls): each pending pair is tagged
as the run's (recorded between `begin_run` or `begin_call` and
`note_call`) or the person's (`JournalBuffer::take_by`), and each side
takes back only its own. A failed run takes back its calls' edits
beneath the person's later ones and leaves those (a task's feature
stays); Cancel takes back the task's and leaves the run's; OK closes the
task's as a step of the person's; a run that ends while the task is open
takes its calls in it into its one step. Where the later side's edits
touch what the earlier side's touched (`history/touches.rs`: the same
feature, body, setting), they were made on them: a failed run keeps its
edits (`RolledBack::kept`, handed to the task), and Cancel or OK takes
the run's edits with the task's. A run opened inside a task is part of
it (Cancel takes it back too, OK keeps it in the task's step).
`history::assert_cancel_takes_back` is the tasks' shared test. A `TaskInfo::stepwise` task (the
sketcher's editing session) is the exception: each edit in it is an undo
step of its own. A tool can make its gesture one step before the pointer
is let go and take back exactly that step: every closed step has a
`JournalEntryId` (`OpJournal::note` answers it, `undo_entry_id` names the
newest, kept through undo and redo), the hook context carries the newest
as `ctx.undo_entry`, `HostRequest::CloseToolGesture` closes the step
through `close_gesture` and `UndoToolGesture` undoes `entry`
(`OpJournal::undo_entry`, refused unless it is the newest step, while a
script's run is open, or while undo itself is blocked), each answered
to the bench at once through `BenchTools::tool_history`.
