# Assembly bench

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
  and the pull halved back to contact; the release records `asm.place`,
  carrying in `turns` the coupling counts it changed; Escape, a tool or
  lost focus puts the bodies and counts back; `session.rs` keeps the
  task, picking, held body and handles per tab),
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
  joint settings and moving a body by numbers (`panel.rs`, the dispatch,
  with one module per panel or group under `panel/`: `joint`,
  `coupling`, `moving`, `interference`, `explode`, `motion`, `copies`
  (with replacing a body and rigid groups) and `parts` (with mass);
  every panel is one `panel::Form` walk, `<panel>_walk`, declaring the widgets
  `task_widgets` hands `bench_panel` and, given an event in
  `task_event`, naming the edit its field stands for (an `<Panel>Edit`),
  which `<panel>_edit` makes through the bench's own commands,
  `asm.set`, `asm.place` and the rest, where one makes the change; `task_turn`, egui-free like the rest, handles
  OK and Cancel and collects work done away from the window, which
  `busy` keeps frames coming for, a drive's sweep stepping in
  `on_frame`, the parts list's Copy a `HostRequest::CopyText`; only
  `draw_task_panel` touches egui; `panel_tests.rs` checks each against
  its command). Solves run inside the
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
  `lib.rs` holds its state and the `Workbench` impl; `registration.rs`
  (tools), `input.rs` (input and each frame's work), `tasks.rs` (the
  task a feature opens), `menus.rs`, `parameters.rs` (a drive's and a
  joint kind's numbers), `checks.rs` (interference, mass and motion
  checks), `frames.rs` (exploded views, motion frames, `sweep_frames`)
  and `viewport_out.rs` (what the view draws) the rest.
