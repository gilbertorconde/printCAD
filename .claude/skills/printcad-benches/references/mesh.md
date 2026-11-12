# Mesh bench and surface textures

- `workbenches/wb_mesh`: the Mesh bench (`docs/MESH.md`). A
  `MeshFeature` (kind `wb.mesh`, on the scan's body) holds one
  `kernel_api::MeshOp`; `Workbench::mesh_plans` lists each body's steps in
  order up to its tip, and the host's `app/mesh_histories.rs` builds a
  changed plan from the body's mesh start (`Document::mesh_start`, kept by
  the first step's `set_mesh_start`) on a thread through
  `KernelQueries::build_mesh_chain`, the result set as the body's
  geometry (derived), a failure on its feature's row; a start no plan
  uses is the body's mesh again. A marking is taken to the start's
  vertices by position (`marked_on_start`). Deviation of the edit
  (`mesh.deviation`, a toggle and a `ViewportOutput::scalar_maps` map measured on a thread)
  measures the mesh now to its start, a triangle whose middle lies off the
  start marking its corners added (grey). Commands `mesh.islands`,
  `mesh.delete`, `mesh.flip`, `mesh.cut`, `mesh.fill`, `mesh.smooth`,
  `mesh.offset`, `mesh.reduce`, `mesh.boolean`, `mesh.thicken`,
  `mesh.inspect`, `mesh.result` and `mesh.set`. `solids.rs`: Boolean
  (`MeshOp::Boolean`, its operand a `kernel_api::MeshOperand` snapshot
  taken when the step is made: a body's solid as native text with its
  placement, or the bounded solid a feature's owner offers through
  `Workbench::feature_solid`, kept per evaluation as
  `Document::feature_solid`; an Inspection element's primitive over its
  drawn extent) and Shell (`MeshOp::Shell`, hollow when closed, thickened
  when open); `mesh.trim_to_scan` cuts a solid body by the scan's break
  (`KernelQueries::trim_to_scan`, the kernel's `cut_by_mesh_region`) and
  lands the result through `replace_body_shape`, or on a new body beside
  a body built from features, which is hidden; `mesh.preview_cut` and
  `mesh.export` (`KernelQueries::write_mesh`) record nothing. Declared task controls edit each step live with OK/Cancel,
  listing triangle counts and the kernel's movement or verified reduction
  measurements. `Document::store_mesh_build` keeps these derived reports
  and source identities by the full plan; desktop and headless builds
  share it. A marking follows moved vertices through the source identities, a
  reduced vertex marked when every start vertex merged into it is
  (`Editable::simplify` keeps them).
- `surface_texture`: patterns pressed into chosen faces of a mesh for
  printing (`Texture`: pattern, projection, tile, depth, turn, inward,
  keep flat; `apply` welds, splits the chosen faces' triangles and their
  neighbours along shared edges so no crack opens, then moves each point
  along its normal, the rim fixed). The document keeps them on the body
  (`Body::textures`, `FaceTexture` with `FaceKey`s found by name like face
  colours, op `SetBodyTextures`, `doc.set_textures`); the app's
  `app/textures.rs` resolves faces and pictures (`Pressing`; a linked
  copy with none of its own takes its source's, `textures_of`, a mirrored
  copy pressing them into its mesh mirrored back and mirroring the
  result), makes the view's preview on threads (`drive_texture_previews`:
  in the body's own frame, drawn as a `MeshInstance`, one per
  `preview_key` so a source and its copies share it; the last one shown
  while a newer is made) and hands export a `finish` per body
  (`ExportBody::finish`, applied in the body's frame before placement);
  `ui/texture_task.rs` is the task. `docs/TEXTURES.md` is the guide.
