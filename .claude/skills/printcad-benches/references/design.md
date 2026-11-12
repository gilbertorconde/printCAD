# Design bench

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
  for error attribution; `plan_feature` hands each family to its module in
  `build/`: `extrude`, `sweeps`, `dressup`, `patterns`, `bodies`, `holes`,
  its tests in `build/tests/` by the same families). `split.rs`: the body menu's Split… and
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
  `lib.rs` holds its state and the `Workbench` impl; `registration.rs`
  (tools and their keys), `input.rs` (clicks, handles, typed values),
  `making.rs` (what a tool reads of the selection, `create_feature`,
  `take_base`), `menus.rs` (body and row menus, Copy and Paste),
  `features.rs` (what it answers for its kinds) and `viewport_out.rs`
  (what the view draws) the rest.
