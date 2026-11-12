# Inspection bench and metrology

- `workbenches/wb_inspect`: the Inspection bench (`docs/INSPECT.md`).
  `lib.rs` holds its state and the `Workbench` impl; `input.rs` (clicks,
  arming, picks), `registration.rs` (tools), `viewport_out.rs` (marks,
  labels, maps, status), `features.rs` (what it answers for its kinds),
  `making.rs` (elements, constructions, dimensions, tolerances),
  `mapping.rs`, `sectioning.rs`, `construction_export.rs` and
  `landing.rs` (alignments, the best fit landed) the rest.
  An `Element` (kind `wb.inspect`, on the scan's body) is a shape fitted
  to the scan's marked points: `ctx.marking` (`MarkedSurface`, the
  host's marking of the tab, `app/marking.rs`) read against the body's
  `local_geometry`, asked of `KernelQueries::fit` (`FitRequest`,
  `FitAnswer`; `kernel_ogeom/src/fit.rs` over the kernel's least-squares
  fits, outliers past `sigma_cutoff` left out); the element keeps the
  marked runs and the answer, drawn as `passive_geometry` lines; a
  `method` (`kernel_api::FitMethod`: minimum zone, inscribed,
  circumscribed, through `KernelQueries::zone_fit` over the kernel's
  `fit::zone`) refits the points least squares kept, and every element
  keeps its points' minimum zone (`Fitted::zone`), the form its form
  tolerances read. With
  nothing marked a fit tool arms (`armed`) and a click on a scan grows
  the surface from the point under the cursor, or traces a circle or
  line along the sharp edge by it (`grow_at`, `KernelQueries::grow`, the
  kernel's region growing over the body's local mesh, its coincident
  vertices welded first since the importer splits them at creases;
  `inspect.grow`). A
  `Dimension` (`dimension.rs`, kind `wb.inspect.dimension`, dependent on
  its elements) reads a distance, angle or diameter off them each time
  it is asked, never stored, so a refit moves it; its nominal, plus and
  minus are `parameters`, and a band gives PASS/FAIL, drawn as labels
  while the bench is active (`viewport_out::spread` keeps them apart). A
  `Construction` (`construction.rs`, kind `wb.inspect.construction`) is
  a point, line, plane or circle made from other elements, likewise
  worked out on each read (`making::built_at`, into an `Element` dimensions read
  as a fitted one; a point is a sphere of radius zero inside the crate,
  answered to scripts as `Point`). Deleting an element takes what is
  made or measured from it. A `Comparison` (`compare.rs`, kind
  `wb.inspect.comparison`, on the scan) keeps a reference body and the
  map's settings; `maps.rs` measures its distances on a thread
  (`KernelQueries::signed_distance`, `kernel_ogeom/src/deviation.rs`: a
  solid meshed finely first, coincident vertices welded) keyed by the
  bodies' revisions and placements, and `ViewportOutput::scalar_maps` hands
  them to the host with a `MapLegend`; the reference draws see-through
  (`ViewportOutput::faded`); `hotspots.rs` groups the points out of tolerance into
  patches (labelled on the part, `inspect.hot_spots`) and `export.rs`
  writes the distances (`inspect.map_export`). `volume.rs` reads the scan against a solid
  reference as material on a grid of cells (`KernelQueries::volume_difference`,
  the kernel's `volume_difference`), missing and excess regions with volume
  and depth, labelled on the part while the comparison keeps a
  `volume_cell` (`inspect.volume_errors`, measured on a thread through
  `Maps::request_volumes`). An `ElementMap`
  (`element_map.rs`, kind `wb.inspect.element_map`) maps the scan
  against one element's ideal shape within its drawn extent, worked out
  in closed form (`inspect.element_map`). A `Pin` (`pins.rs`, kind
  `wb.inspect.pin`) keeps a point on the map shown on a scan
  (`shown_map`), its label the value there, and the cursor over a mapped
  scan reads it (`hovered_reading`; the overlay hooks' context carries
  the hovered body, point and face). A `Section` (`section.rs`, kind
  `wb.inspect.section`, `inspect.section`) keeps a plane in the scan's
  frame, an offset and a stack count; its cuts come from
  `KernelQueries::mesh_sections` (`kernel_ogeom/src/orient.rs` over the
  kernel's exact mesh section), worked out again whenever the scan or
  the section changes and kept by that key (`section::Cuts`), drawn as
  `passive_geometry` with the kernel the bench last saw. A `Shadow`
  (`shadow.rs`, kind `wb.inspect.shadow`, `inspect.shadow`) sits on a
  body of its own: `derive_on_geometry` asks `KernelQueries::mesh_shadow`
  (the kernel's mesh shadow and `fit_outline`, lines and arcs within a
  play band) whenever the scan, its placement or the settings change,
  keeping the outline in the document's frame as `made`, and the bench's
  `rebuild_jobs` extrude it into the body's block. `inspect.export_step`
  (Save elements as STEP) writes elements, sections and outlines as
  construction geometry, `construction_export::construction_items` through
  `KernelQueries::write_construction_step`. `pmi.rs` answers
  `Workbench::semantic_pmi` for each Comparison's reference: tolerances,
  datums lettered in first-use order and dimensions, their face probes
  placed from the scan into world coordinates. A `WallThickness`
  (`thickness.rs`, kind `wb.inspect.thickness`, on any body; `inspect.walls`) is read by
  `KernelQueries::thickness` (rays or spheres, a solid exactly) through
  the same threaded `Maps::run`, thin painted hot. An `Alignment` (`align.rs`, kind
  `wb.inspect.alignment`) records how the scan was moved and the
  placement before and after; making one sets the scan's placement. Best
  fit runs `KernelQueries::register` (the kernel's point-to-plane ICP, a
  coarse search over principal-axis matchings then a refinement) on a
  thread, applied in `on_frame`. Datum, automatic and symmetry alignment
  (`inspect.align_datums`, `inspect.auto_align`, `inspect.use_symmetry`,
  `inspect.align_points` through `metrology::rigid`,
  `align_commands.rs`) use ordered elements or
  `KernelQueries::orient`/`symmetry_plane`. Commands `inspect.fit`,
  `inspect.grow`, `inspect.construct`, `inspect.dimension`,
  `inspect.tolerance` (`tolerance.rs`, kind `wb.inspect.tolerance`: form
  from an element's band, orientation through `metrology::orientation`
  against a datum element; position, profiles, runout and symmetry
  in `tolerance_read.rs` over `metrology::gdt`, a datum frame of up to
  three references, a profile measured against the comparison's
  reference through `KernelQueries::signed_distance` on a thread and
  kept by its inputs; double click opens `tolerance_task.rs`, its panel
  declared widgets that `walk` lists and applies a `PanelEvent` to through
  `bench_api::panel::Form`, drawn
  by `bench_panel` as a package's is), `inspect.compare`,
  `inspect.best_fit`, `inspect.move`, `inspect.reset_alignment` and
  `inspect.report` (`report_of`, as JSON, or CSV and text through
  `report.rs`; the Save report tool hands the CSV to `SaveFile`). Selecting a
  feature on a mesh body moves no tip (`move_in_time_to`).
- `metrology`: standalone std-only measurement arithmetic for scan
  inspection (RFC 0004): `stats` (summary, percentiles, histogram, share
  within a tolerance; `NaN` is unmeasured), `form` (peak-to-peak,
  smallest and largest diameter), `orientation` (parallelism,
  perpendicularity, angularity, coaxiality, concentricity zones), `ramp`
  (jet and the public-domain perceptual ramps with end caps; where a
  value lands on a signed or banded scale) and `design` (the round value
  a measured size was designed at). `profile` (an outline's radius against angle, peaks by prominence;
  `sketch.image_profile`). `rigid` (Horn's closed-form fit of point pairs). `gdt` (a datum
  reference frame from up to three planes or axes, position, profile,
  runout, symmetry zones and the material bonus). `shrink` (a print's per-axis stretch from distances along normals,
  `inspect.compensation`). Copyable like `sketch_solver`.
