---
name: printcad-sketcher
description: "The sketcher and its maths: sketch_solver (problem, compile, LM solve, freedom and diagnosis), sketch_model (the f64 model and the f32 boundary, constraints and solving, profiles and regions, text layout, generated profiles), wb_sketch (tools, input, panels, overlays, external geometry, DXF and image tracing), and image_edges (subpixel edges and objects in images). Use when working in crates/sketch_solver, crates/sketch_model, crates/workbenches/wb_sketch or crates/image_edges, on constraints, snapping, dimensions, profiles, sketch tools or sketch commands. Not for the Design features built on a sketch (use printcad-benches) or the snapping-reuses-ids invariant, which is in CLAUDE.md."
---

# printCAD: sketcher

The invariants in `CLAUDE.md` still bind here; this skill never weakens one. Sketch endpoint snapping reusing point ids is the one that bites most here. External geometry's app wiring is under Placeholders in `printcad-app-shell`; sketcher end-to-end tests are in `printcad-testing`.

## Crates

- `sketch_solver`: standalone std-only sketch mathematics: ordered typed
  `problem`, semantic `compile`, `curves`, `spline`, `residual`, LM `solve`,
  `sparse` (the Jacobian by blocks of the variables equations join: each
  residual's `reads` names its variables; differences, the LM step, rank
  and held variables all worked out per block, so a residual kind must
  list every variable it reads, which `sparse`'s tests check per kind),
  rank/DoF `freedom` and `diagnosis`, `soft` (measured points settling
  what the constraints leave free), `parts` (a problem by its parts, the
  elements no point or constraint joins, each compiled once and kept in
  a `PartCache` while equal, solved and ranked together to the bit of
  the whole system; what crosses parts is listed in its module comment,
  and a change there needs `parts_tests` to hold); optional serde. Its
  complete directory can be copied to another consumer without workspace inheritance.
- `sketch_model`: the sketch as data, no egui, in the layer beside
  `bench_panel`; Design, Surface, project import and the fixtures depend
  on it rather than on `wb_sketch`, and `check-workbench-seam.mjs` keeps
  it out of the host as part of the sketcher. Its model (`sketch.rs`:
  points, radii, constraint values, `SketchPlane`) is `f64`, stored as the
  solver answers it; `boundary.rs` is the one place values cross to and
  from the `f32` of screens, GPU meshes, picks and the package contract
  (incoming singles widened through their shortest decimal). `feature.rs`
  (`SketchFeature`, its supports, `KIND`), `geom2d.rs`
  (intersection/sampling math), `shape.rs` (an element as a shape:
  `distance_to_element`, `distance_within`), `curves.rs`, `conic.rs`,
  `spline.rs`, `measure.rs`, `change.rs` (`SketchChange::between`: the
  elements and constraints added, removed and changed, by id;
  `SketchFeature::patch_over`, the patch an edit records instead of the
  whole sketch, through `wb_sketch::placement::write_sketch_data` from
  the bench's and the commands' stores; a sketch with text is written
  whole), `solver.rs` (sketch_solver adapter, storage
  write-back and application diagnostics; every solve and measure goes
  through `Prepared`, its parts from the thread's `PartCache`, a
  diagnosis alone compiling the whole problem; `locked_elements`, through
  `Parted::locked` (`sketch_solver::freedom::locked` by parts), the
  elements nothing left free can move, drawn in `SketchPalette::locked` until the whole sketch is fully
  constrained, cached by the sketch it was worked out on; a text's
  letters, a derived focus and a refitted spline's controls follow what
  they are laid out from; `free_points`, Select degrees of freedom, is
  every point it leaves out, at any size), `profile.rs` (closed-wire
  extraction; ends tied by a Coincident constraint join as a shared point
  does), `regions.rs` (the planar arrangement: curves split where they
  cross, touch or branch, every bounded face a `Region` named by the
  elements round it and the side of each it lies on;
  `profile::extract_regions` outlines the ones named, which Design's Pad,
  Pocket, Revolution and Groove keep in `regions`, chosen by clicks in the
  view while their task is open, `wb_design/src/region_pick.rs`, or the
  task's checks; `sketch.regions` lists them), `text.rs` (text layout and
  fonts), `generator/` (the generated profiles' numbers and outlines; the
  `sketch.generator` command and panel are `wb_sketch`'s), `params.rs`
  (dimensions as formulas set them), `render.rs`, `repair.rs`,
  `operations.rs`, `style.rs` (icons and names per element/constraint
  kind).
- `workbenches/wb_sketch`: the sketcher bench over `sketch_model`:
  `lib.rs` (its state and own methods; the `Workbench` impls are in
  `bench.rs`), `tools.rs` + `tools/{draw,transform}.rs` and
  `tools/modify/{corner,trim,general,offset}.rs` (state machine), `snap.rs` (`resolve`, the cue's and the click's snap: the sketch
  looked up into a `Scene`, kept for the sketches the frame shares
  through `snap::share` and built per query for any other, the kinds in a fixed order,
  ties settled by `Rank` from what the candidates are, never by ids,
  which a replay draws anew; `snap::Lookup` the one lookup a click's
  or a preview's point and curve snaps share; `examples/snap_bench.rs` times it, an edit and a label drag on a large
  sketch), the line tool's drag from its end (`LinePress`, `step::line_arc_click`:
  the polyline's tangent arc, a `sketch.draw` point with `arc = true`),
  the line and polyline runtime (`chain.rs`: a click resolved once,
  `chain::resolve` after `step::land`, into the edit it proposes,
  `propose`, which the preview draws and the click applies, `apply`
  checking every point and curve it names before its first write; each
  segment closes its own undo step through `CloseToolGesture`, kept in
  `chain::Links`, so Backspace (`UndoToolGesture`) or the host's undo puts
  the gesture before it and an outside edit that takes its start ends it,
  `follow_history`),
  a scan's section (`scan.rs`: every visible scan cut by the sketch
  plane through `KernelQueries::mesh_sections`, kept by plane and scans
  in `scan_guides` and drawn dashed while editing; `sketch.fit_scan`
  fits each chain with `KernelQueries::fit_chain_2d`, the kernel's
  `fit_segments_2d`, and adds the pieces through the DXF path,
  `dxf::add_drawing`, so ends join; `drawing.rs` writes the
  sketch at true scale as layered R2000 DXF (`sheet`, through
  `KernelQueries::write_dxf_drawing`: elements, construction, picture
  edges thinned to 0.01 mm and text on layers of their own, mm or inch),
  SVG or CSV of elements and dimensions, each with a `traceability`
  line, `sketch.export_dxf`/`export_svg`/`export_csv`, Save as
  DXF/SVG/CSV; `candidates.rs` reads each size's and the relations
  drawn geometry nearly keeps, `sketch.candidates`, `sketch.design_values`;
  fitted pieces keep their slice points as `Sketch::soft`, which
  `solver::solve` hands `sketch_solver::soft::solve_soft`: the constraints
  met exactly, the points settling what they leave free),
  a picture's scale (`ImageScale`: given, nominal from the file's dpi,
  `images::dpi`, or calibrated by a distance or `sketch.calibrate_image`'s circle;
  an UNCALIBRATED label until then; `stretch` for a height scaled apart,
  `sketch.calibrate_axis`; scanner profiles in the configuration folder's
  `scanner_profiles.json`, applied on import by the file's dpi; `mirrored`,
  `sketch.align_image`, `sketch.turn_image`; `sketch.bolt_circle` through
  circles' centres),
  a picture's edges (`picture_edges.rs`: `image_edges` on a thread per
  asset, kept in pixels and drawn as guides; `sketch.fit_edge` grows a line or
  circle from the click along the edge while each point keeps to the shape;
  a point tool's click lands on the nearest edge at subpixel in `landing`
  (`edge_snap`, `nearest_on_edge`) unless `ctx.alt_down`, with a 4× loupe
  through its viewport's `images`),
  reference pictures (`images.rs`, `Sketch::images`, the file a document
  asset, decoded once each, shown through `ViewportOutput::images`
  as egui textured quads under the lines; `sketch.image`, `sketch.set_image`,
  File › Import for PNG and JPEG),
  the profile's regions shaded while editing (the viewport's `meshes` over
  `KernelQueries::profile_mesh`, cached by profile, a moving drag keeping its start's until release) and out of it (the
  passive geometry's `regions`, one `PassiveRegion` per region with its
  `FeaturePart`, the plain profile's regions stronger, which the host
  meshes once per revision and draws see-through behind the lines, lit
  in the hover or selection paint), loose ends ringed,
  a closed sketch's parts picked with no task open (`closed_parts.rs`:
  `BenchFeatures::pick_part` answers the curve within reach or the region
  under the cursor, `part_lines` a picked curve's lines; the host keeps
  them as `DocumentSession::selection.parts` beside the selected feature,
  a click taking a region alone and Ctrl adding regions or curves, a
  region behind the face the pick found left alone, and hands them to
  benches as `ctx.selected_parts`; Design's Pad, Pocket, Revolution and
  Groove start with the picked regions, a revolution or groove about a
  picked straight curve, `region_pick::picked_regions`/`picked_axis`),
  `overlay.rs` (screen-space rendering while editing), `glyphs.rs` (constraint icons and dimension layouts),
  `constrain.rs` (which constraint a toolbar action creates for the
  selection's shape; a tool the selection does not fit yet arms in
  `model_picks.rs`, taking sketch clicks and the host's edge picks as
  operands, projecting the edges as guides, reusing what the sketch
  holds, and recording `sketch.external` then `sketch.constrain`),
  `panel/` (the task panel, declared as blocks, `task_widgets`, drawn
  by `bench_panel` and answered in `task_event` through the
  sketcher's commands; `PanelState` keeps what each row and button stood
  for; `sections.rs`, `lists.rs`, `text.rs` and `builders.rs` declare it,
  `events.rs` reads it back; a generated sketch's panel is `generator/panel.rs`, declared the
  same way), `dimension_task.rs` (a dimension's value, formula or new
  variable in a floating card by its label, `dimension_card`: opened by
  a new dimension, which waits in the card with the calls that make it
  until OK (`DimEdit::made`), a double click on a label or a value typed
  into its row; the draft worked out on a copy of the document through
  `evaluate_with_retained_parameters` and solved, shown through
  `displayed_sketch`; OK records the dimension, switches, variable,
  formula and value, and the host closes them as one undo step),
  `corner_task.rs` (what a click leaves to set: a fillet or chamfer
  corner, a rounded rectangle's radius, a frame's wall, an offset, a
  text, waiting in the task panel's Tool section, `pending_widgets`,
  whose fields edit it live: the click replayed by `step::click` (a
  text by `TextAt::add`, its font read by `text::preview_font` without
  keeping it) on the sketch as it found it, shown through
  `displayed_sketch` and never stored until accepted; only its Cancel
  drops it, while OK, Enter, Escape, a click in the view, another tool
  or finishing the sketch record it as the `sketch.draw` or
  `sketch.text` call, one undo step). `docs/SKETCHER.md` is the user guide.
- `image_edges`: standalone std-only subpixel edge chains in grey images
  (Gaussian, Sobel, ridges across the edge, thresholds from the image's
  noise with hysteresis, a parabola across the gradient, chains that know
  their brighter side); better than a tenth of a pixel on synthetic edges.
  `objects`: parts split from the ground by Otsu's level, measured (area,
  holes, centroid, calipers, long axis, outline); `sketch.image_objects`.
  Copyable like `sketch_solver`.
