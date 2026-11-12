# Known approximations / roadmap

- `TriMesh.faces` names the kernel face each triangle was cut from, so a click
  selects a whole face, a bore or a fillet as much as a flat side
  (`doc_render::face_pick::face_submesh`). A mesh without faces (a sketch, a datum, a
  document saved before they were recorded) falls back to the plane through
  the hit. References find their faces by name (`kernel_api::naming`):
  the chain names every face as it builds (`kernel_ogeom::naming`: a
  feature's walls after the sketch elements they were swept from, which
  the sketcher's profiles carry as `ProfileWire::names`; its ends after
  where they stand; other new faces by surface kind and facing; a face a
  later op splits, trims or merges after the faces the kernel's history
  says it came from (every `Built.history` an op's kernel calls return
  is recorded, `naming::record`, and followed back), a face generated
  from an edge (a fillet's round) after that edge's two faces, and where
  the history says nothing after the faces it lies on or covers; the
  faces an op left alone recognised by a `Print`), each op under its
  feature's name (`ChainRequest::tags`, the worker's tags). The
  mesh carries `face_names` and `edge_faces`; a pick keeps them
  (`FaceRef::name`, `EdgeRef::faces`, `FacePick`, `EdgePick`, datum
  anchors, `FaceSupport` of a sketch placed on a face), and a rebuild finds
  a named face or edge by name first (`naming::find_face`/`find_edge`,
  reading the running solid's names the chain sets for the op), falling
  back to the stored point + normal. A reference whose names are lost is
  found by position only within a tenth of the solid's diagonal
  (`probe::position_reach`, else a `ChainError` saying it is gone), and
  the op that took it is warned (`SolidBuildResult::warnings`, kept as
  the derived `FeatureNode::warning`, the row's Warning chip and the
  scripts' `warning`); a probe's or projection's warning
  (`probe_warnings`, `projection_warnings`, by their place in the list)
  goes to the feature that asked it, a datum or a sketch. Edges picked on
  another body (a surface step's `CurveSource::BodyEdge`, a pipe's
  `EdgesOf`) are found by position, `dressup::picked_edges_elsewhere`,
  since the chain's names are its own body's. A sketch placed on its body's face
  follows it: Design's plan asks where the face stands at the
  sketch's place in history, and `Workbench::derive_on_solid` moves the
  plane from the placement it recorded. One on a face a borrow lends
  (`FaceSupport::lent_by`) is answered from the borrow instead
  (`answer_lent_faces`, which finds the face in the lender's mesh by
  name); `SketchAttachRequest::face_origin` says which a face is, and a
  face of another body is not followed at all. `TriMesh.edge_ids` names the kernel edge of every
  outline segment (the tessellator draws the outline from the kernel's own
  edges, each to the chord the faces agreed on; triangle boundaries are only
  the fallback), so a click within a few pixels of an outline picks the
  whole edge (`app/edges.rs`: hover, Ctrl-additive selection, highlight line
  bodies, the hover card's edge length, the measure tool's snap). Benches
  see picked edges as `ctx.selected_edges` (point, direction, length,
  faces); Design's fillet and chamfer store them as `EdgeSel::Edges`
  picks the kernel resolves through `EdgeSelection::Picked`.
- "Through all" derives its length from the base solid's bounding box.
  Up to face (`ExtrudeTermination::UpToFace`, the base's face nearest the
  pick), to first and to last trim a long prism by the half-space of the
  target face's whole surface, pushed out by the offset along an offset
  surface, so flat and curved targets alike stop exactly on the face.
- Helix: the height + turns + growth mode sends `SweepKind::Helix` its
  turns and growth per turn (in place of the cone angle); at height 0 it
  is a flat spiral. A subtractive helix with "keep inside" is
  `BooleanOp::Common` (a pattern cannot repeat it).
- Pipe: `SolidOp::Pipe` carries a `PipeFrame` (rotation-minimizing,
  Frenet, auxiliary path, binormal; files with the old `frenet` flag read
  as the first two), a `PipeCorner` and further `sections`; an open path
  runs from the end its profile sits by (`sketch_spine`). `pipe_tool`
  (`ops/loft_pipe.rs`) hands the frame to the kernel's `PipeLaw` and the
  corner to `PipeCorners` (`make_pipe_shell_with`), and extra sections to
  `make_pipe_sections`, the profile placed at the path's start and each
  section where it stands. A revolution up to a face whose plane holds its
  axis stops at one angle; any other face goes to `make_revolution_until`.
  A thickness's arc join is the kernel's `Join::Arc`.
- Hole threads: `wb_design/src/hole_tables.rs` holds the thread standards
  (ISO metric coarse and fine, UNC/UNF/UNEF, BSW/BSF, BSP G and Rc, NPT),
  their classes, the ISO 4762/10642 seats and the user's `hole_cuts.json`
  profiles (read once at start, copied into a hole when picked, so geometry
  never depends on the machine). A hole's `thread` is a `ThreadSpec`
  (standard, designation, class, hand); `metric_index` in old files
  deserializes into it. A threaded standard hole drills its tap diameter
  (a taper thread its minor diameter at the face, narrowing 1:16),
  clearances otherwise; "Modeled thread" also cuts the standard's groove
  (60° or 55°) along a helix out to the major diameter, along a cone for a
  taper (`thread_cut` in `wb_design/src/build/holes.rs`). A cosmetic thread
  (`cosmetic_thread`) draws a helix instead, and its designation reaches
  the host through `Workbench::cosmetic_threads` (`ThreadCallout`, kept by
  `DocumentService::evaluate` as derived `Document::cosmetic_threads`),
  which the parts list's Threads column, `app/annotations.rs`'s callouts
  and a STEP export's callouts (`export.rs::body_notes`) read. Drill points and
  counterdrill cones are kernel cone primitives cut at each centre.
  `docs/HOLES.md` is the user guide.
