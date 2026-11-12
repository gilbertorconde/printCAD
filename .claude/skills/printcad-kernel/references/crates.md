# Kernel crates

- `kernel_api`: pure data contract (TriMesh with per-triangle kernel face ids, ProfileWire w/ ellipse+B-spline
  segments, `SolidOp` = sweep/loft/pipe/primitive/dress-up/transform/boolean,
  ExtrudeTermination, TessellationSettings, ChainError, `PointMatch`: the
  key a mesh point hashes by when welding or matching corners, exact or a
  grid). No geometry code but `arc`: which three points make a profile
  arc (the kernel's collinearity ratio, `COLLINEAR_SINE`, and a radius
  limit, `MAX_RADIUS`, past which the kernel's circle misses its points)
  and where it runs, the one rule the sketcher's regions and drawing
  tools, Design's borrow and the kernel adapter's profile hold arcs to.
  `JOIN_MM` (1e-3) is how near two curve ends are one point across the
  contract: a profile wire's segments, the kernel's section runs, and the
  ends Section to sketch, borrow and the sketcher's model picks read off
  a solid's edges. The sketcher's own join, `curves::END_SLACK` (1e-4:
  regions, projected ends, DXF), is held no wider at compile time, so
  what a sketch closes the kernel builds.
  Every weld in `kernel_ogeom` goes through
  `mesh_intake::intake` with a named match (`SAME_POINT` for a solid's
  faces, scans read and exports, so a mesh is closed in all or none;
  `HISTORY_POINT`, exact, for a mesh history's start); benches and
  `surface_texture` key by `PointMatch` with a constant of their own.
- `kernel_ogeom`: pure-Rust kernel adapter. STEP and IGES import share one
  path after the read (`import.rs`, reader chosen by extension, `is_iges`);
  IGES solids and closed surface groups become bodies, open sheets are left
  out with a log line. STL, OBJ, 3MF, PLY, glTF and VRML (`mesh.rs`; glTF and VRML are in
  metres and scaled) import as **mesh bodies**: triangles and no shape snapshot (`Document::is_mesh_body`),
  welded where normals agree within 30° and outlined at creases and holes;
  they draw, hide and pick and take no features. "Convert to solid" (tree
  and viewport menus) records `RequestMeshSolid`, a history barrier, and
  `drive_mesh_solids` derives the B-rep with the kernel's `solid_from_mesh`
  as the mesh is (coplanar triangles merged into faces, curved stretches
  left as facets with every vertex kept, `ShapeHealth::faceted`; a mesh
  that does not close becomes an open shell, said in the log), after which
  the body is an ordinary imported solid. "Refine shape" (`RequestBodyRefine`,
  a history barrier, offered by `Document::can_refine` while the body is
  faceted and has no base) has `drive_shape_refinements` run the kernel's
  `refine_solid` (`mesh::refine_blob`), which rebuilds the cylinders, cones,
  spheres and tori among the facets; a refine that fails or would not
  close leaves the facets and is not asked again that session. A faceted
  body's row carries a FACETED badge, and its conversion logs a warning,
  saying features on its curved areas may fail until it is refined. The files' annotations (`annotations.rs`: PMI callouts,
  undrawn semantic dimensions/tolerances/datums, datum targets, notes)
  leave as `ImportedModel.annotations`, one per body a callout describes,
  placed with that body's occurrence, text formatted (`Ø 35 ±0.2`); IGES
  levels and groups as `ImportedBody.layers`. The app keeps them on the
  import's nodes (`ImportedObjectNode::annotation`/`layers`, an
  `Annotations` group under the model's root) and `app/annotations.rs`
  draws them as screen-space lines and labels (`rendering.show_annotations`,
  View › Annotations), placed and hidden with their body. STEP import builds bodies from
  the document's **placed occurrences** (`Document::occurrences_of`), never
  from `import.solids`; the latter are part-local, so an assembly built from
  them puts every part at its own origin. The node walk mirrors the kernel's
  preorder flatten so the n-th part leaf is the n-th body. (`import.rs`) +
  `execute_solid_chain` (`chain.rs`): one in-memory ogeom `Model` per chain,
  the running `Shape` threaded op-by-op (`ops/{sweep,primitive,dressup,
  loft_pipe,pattern}.rs`); native-format text blobs only at the boundaries
  (result out, `SolidOp::Boolean` tool in, via `io::native`). Errors carry the
  failing op index (`ChainError`). Profile wires group by containment:
  nested = holes, disjoint = separate solids (compounded when regions stay
  disjoint). Patterns re-run the tool op under the transform rather than
  instancing. A pattern with a path step (Design's Path Pattern, or a
  Multi Transform's Path step) plans `SolidOp::Pattern`, whose steps the
  chain turns into transforms where the op stands
  (`ops/pattern_steps.rs`: a sketch's chain, or an edge of the running
  solid found by its `EdgeProbe`, walked by arc length, an aligned copy's
  turn carried along it), so the plan stays a pure function of the
  document; `KernelQueries::pattern_transforms` answers the same for the
  task's occurrence dots. A kernel-side gap gets a test marked
  `#[ignore = "kernel: …"]`; grep for `kernel:` in `tests/`
  before assuming a feature is wired wrong.
- `crates/kernel_ogeom/examples/import_bench.rs` prints the phase breakdown of
  an import; reference timings live in the import-performance memory.
- No system CAD libraries needed: the ogeom kernel is pure Rust. The
  workspace `Cargo.toml` pins it by `rev` on its repository's main; a new
  kernel fix is taken by editing that `rev` (the kernel's thread sends the
  commit, issue and final API after each merge). Only `kernel_ogeom`
  depends on it, and nothing published to crates.io does. A commented
  line in `[patch.crates-io]` points at a local checkout for kernel dev.
