# Surface bench

- `workbenches/wb_surface`: the Surface bench (`docs/SURFACES.md`).
  `SurfaceFeature` (kind `wb.surface`) on surface bodies only:
  `SurfaceWorkbench::takes_surfaces` (a surface body, or one of sketches
  and datums), and Design sends its features off a surface body to a new
  one. `build.rs` turns the body's history into `SolidOp::Surface`
  steps (`kernel_api::SurfaceOp`, `CurveSource`: a sketch's chains from
  `wb_sketch::profile::extract_chains`, open or closed, in the body's
  frame, or a picked `EdgeProbe`); `kernel_ogeom/src/ops/surface.rs`
  builds them, a constructive step adding its sheet beside the body's
  pieces in a compound, Sew joining every face (a closed shell made a
  solid, across a gap when given one), the other steps the kernel's sheet
  operations. A kind the kernel cannot build would carry `waits` (tool
  `planned`, no command), with an ignored test for its gap in
  `tests/kernel/surface_ops.rs` (none today). The free-form scan fit
  (`surface.fit_scan_spline`, `SurfaceOp::FitScattered`: a region flooded
  across triangles turning less than `turn_deg`, its points kept with the
  step) builds through the kernel's `fit_surface_scattered_over`, seeded
  on the nearer of a fitted plane or cylinder. A patch network
  (`surface.fit_network`, `SurfaceOp::FitNetwork`: curves as sources and
  polylines, patches as signed curve numbers or chained sides, the scan's
  points read at plan time) builds through the kernel's `fit_network_chained`,
  every curve sampled to a polyline, the points within the curves' box,
  with optional parameter-correction passes after refinement.
  `surface.fit_scan` fits a plane or cylinder to a scan region (grown
  through `KernelQueries::grow`) and builds it as an extrusion;
  `surface.trim_scan` (`SurfaceOp::TrimByScan`, the scan's triangles in
  the body's frame) cuts the body's sheets by each other with the
  kernel's `split_sheet`, keeps the faces lying on the scan and sews them.
  Commands `surface.*` take `sketches`, `body` (a body or a feature in it)
  and any field by name, refusing a step that misses its inputs
  (`SurfaceFeature::missing`); `surface.set` changes a step's fields and
  curves, `surface.check` returns a body's joins. Check continuity (`surface.check`) asks
  `KernelQueries::continuity` (the kernel's `analyse_blend` over every
  face: gap, crease and curvature jump per shared edge, G2 below
  `G2_PER_MM`) of the selected body and labels the edges through
  its viewport's `labels`. The curvature map and zebra stripes
  (`analysis.rs`) paint the selected body as one viewport mesh
  (`ViewportOutput::meshes`), cached by the body's geometry revision: the map's values from
  `KernelQueries::curvature` at the mesh's vertices, the stripes from its
  normals, each triangle split along the stripes' borders; colours from
  the palette's `analysis_*` and `stripe_*`. Its task panels (`panel.rs`:
  each step's editor, the continuity table, the analyses' settings and
  legend) are declared widgets drawn by `bench_panel`, one
  `panel::Form` walk listing them (`task_widgets`) and, given a
  `PanelEvent`, applying it to the step or display it walks
  (`task_event`).
